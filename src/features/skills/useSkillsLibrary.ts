import { computed, onMounted, onUnmounted, ref } from "vue";
import type { SkillsApi } from "./api";
import type { PackagedSkill, SkillPackage, SkillPackagePreview, SkillResource, SkillResourcePage } from "./types";

export function useSkillsLibrary(options: { api: SkillsApi; pickDirectory: () => Promise<string | string[] | null> }) {
  const packages = ref<SkillPackage[]>([]);
  const loading = ref(false);
  const error = ref("");
  const search = ref("");
  const preview = ref<SkillPackagePreview>();
  const previewPath = ref("");
  const previewBusy = ref(false);
  const importing = ref(false);
  const installing = ref(false);
  const changing = ref(new Set<number>());
  const selected = ref<{ package: SkillPackage; skill: PackagedSkill }>();
  const resources = ref<SkillResource[]>([]);
  const resourcesLoading = ref(false);
  const resourceError = ref("");
  const selectedResource = ref<SkillResource>();
  const resourcePage = ref<SkillResourcePage>();
  const resourceText = ref("");
  const resourceLoading = ref(false);
  let disposed = false, listGeneration = 0, previewGeneration = 0, resourceGeneration = 0, fileGeneration = 0;
  const packageSkillIds = computed(() => packages.value.flatMap(item => item.skills.map(skill => skill.id)));
  const visiblePackages = computed(() => {
    const query = search.value.trim().toLocaleLowerCase();
    return packages.value.map(item => ({ ...item, skills: item.skills.filter(skill => !query || `${skill.name} ${skill.description} ${item.name} ${item.source}`.toLocaleLowerCase().includes(query)) })).filter(item => !query || item.skills.length);
  });
  async function loadPackages() {
    const generation = ++listGeneration;
    loading.value = true;
    error.value = "";
    try {
      const next = await options.api.listPackages();
      if (disposed || generation !== listGeneration) return;
      packages.value = next;
      if (selected.value) {
        const owner = next.find(item => item.id === selected.value?.package.id);
        const skill = owner?.skills.find(item => item.id === selected.value?.skill.id);
        if (owner && skill) selected.value = { package: owner, skill };
        else closeResource();
      }
    } catch (reason) { if (!disposed && generation === listGeneration) error.value = String(reason); }
    finally { if (!disposed && generation === listGeneration) loading.value = false; }
  }
  async function choosePackage() {
    if (previewBusy.value || importing.value) return;
    const generation = ++previewGeneration;
    previewBusy.value = true;
    error.value = "";
    preview.value = undefined;
    previewPath.value = "";
    try {
      const path = await options.pickDirectory();
      if (disposed || generation !== previewGeneration || !path) return;
      if (typeof path !== "string") throw Error("Select one skill package folder");
      const result = await options.api.previewPackage(path);
      if (disposed || generation !== previewGeneration) return;
      preview.value = result;
      previewPath.value = path;
    } catch (reason) { if (!disposed && generation === previewGeneration) error.value = String(reason); }
    finally { if (!disposed && generation === previewGeneration) previewBusy.value = false; }
  }
  function cancelPreview() {
    if (importing.value) return;
    previewGeneration++;
    preview.value = undefined;
    previewPath.value = "";
    previewBusy.value = false;
  }
  async function importPreview() {
    if (!preview.value || !previewPath.value || importing.value) return;
    const generation = previewGeneration;
    const input = { path: previewPath.value, expectedDigest: preview.value.digest };
    importing.value = true;
    error.value = "";
    try {
      await options.api.importPackage(input);
      if (disposed || generation !== previewGeneration) return;
      preview.value = undefined;
      previewPath.value = "";
      await loadPackages();
    } catch (reason) { if (!disposed && generation === previewGeneration) error.value = String(reason); }
    finally { if (!disposed) importing.value = false; }
  }
  async function installBundled() {
    if (installing.value) return;
    installing.value = true;
    error.value = "";
    try { await options.api.installBundled(); if (!disposed) await loadPackages(); }
    catch (reason) { if (!disposed) error.value = String(reason); }
    finally { if (!disposed) installing.value = false; }
  }
  async function setEnabled(skill: PackagedSkill, enabled: boolean) {
    if (changing.value.has(skill.id)) return;
    changing.value.add(skill.id);
    listGeneration++;
    loading.value = false;
    error.value = "";
    try { await options.api.setEnabled(skill.id, enabled); if (!disposed) await loadPackages(); }
    catch (reason) { if (!disposed) error.value = String(reason); }
    finally { if (!disposed) changing.value.delete(skill.id); }
  }
  function closeResource() {
    resourceGeneration++; fileGeneration++;
    selected.value = undefined; resources.value = []; selectedResource.value = undefined;
    resourcePage.value = undefined; resourceText.value = ""; resourceError.value = "";
    resourcesLoading.value = false; resourceLoading.value = false;
  }
  async function openSkill(owner: SkillPackage, skill: PackagedSkill) {
    closeResource();
    const generation = resourceGeneration;
    selected.value = { package: owner, skill };
    resourcesLoading.value = true;
    try {
      const result = await options.api.listResources(skill.id);
      if (disposed || generation !== resourceGeneration) return;
      if (result.skillId !== skill.id || result.packageId !== owner.id) throw Error("Resource response does not match the selected skill");
      resources.value = result.resources;
      const entry = result.resources.find(item => item.path === skill.entryPath) || result.resources.find(item => item.kind === "instructions");
      if (entry) await readResource(entry);
    } catch (reason) { if (!disposed && generation === resourceGeneration) resourceError.value = String(reason); }
    finally { if (!disposed && generation === resourceGeneration) resourcesLoading.value = false; }
  }
  async function readResource(resource: SkillResource, append = false) {
    const selection = selected.value;
    if (!selection || (append && (resourceLoading.value || !resourcePage.value?.hasMore || resourcePage.value.path !== resource.path || resourcePage.value.nextOffset === null))) return;
    const offset = append ? resourcePage.value!.nextOffset! : 0;
    const generation = ++fileGeneration;
    const selectionGeneration = resourceGeneration;
    resourceLoading.value = true; resourceError.value = "";
    selectedResource.value = resource;
    if (!append) { resourceText.value = ""; resourcePage.value = undefined; }
    try {
      const result = await options.api.readResource(selection.skill.id, resource.path, offset);
      if (disposed || generation !== fileGeneration || selectionGeneration !== resourceGeneration) return;
      if (result.skillId !== selection.skill.id || result.path !== resource.path || result.offset !== offset || (result.hasMore && (result.nextOffset === null || result.nextOffset <= offset))) throw Error("Resource page does not match the selected file");
      resourcePage.value = result;
      resourceText.value = (append ? resourceText.value : "") + (result.content ?? "");
    } catch (reason) { if (!disposed && generation === fileGeneration && selectionGeneration === resourceGeneration) resourceError.value = String(reason); }
    finally { if (!disposed && generation === fileGeneration && selectionGeneration === resourceGeneration) resourceLoading.value = false; }
  }
  onMounted(() => { void loadPackages(); });
  onUnmounted(() => { disposed = true; listGeneration++; previewGeneration++; resourceGeneration++; fileGeneration++; });
  return { packages, loading, error, search, preview, previewBusy, importing, installing, changing, selected, resources, resourcesLoading, resourceError, selectedResource, resourcePage, resourceText, resourceLoading, packageSkillIds, visiblePackages, loadPackages, choosePackage, cancelPreview, importPreview, installBundled, setEnabled, openSkill, closeResource, readResource };
}
