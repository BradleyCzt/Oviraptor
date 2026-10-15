#!/usr/bin/env python3
"""Reviewable, stdlib-only evidence selection, privacy scan and archive builder.

Plan/privacy are provisional reads. Do not run pack against the real evidence
tree until its owner confirms all validation writers have finished.
"""
from __future__ import annotations

import argparse
import contextlib
import dataclasses
import fnmatch
import gzip
import hashlib
import io
import json
import os
from pathlib import Path, PurePosixPath
import re
import shutil
import stat
import tarfile
import tempfile
from typing import BinaryIO, Iterator

HERE = Path(__file__).absolute().parent
DEFAULT_SOURCE = Path("/workspace/validation/asset-accuracy-20261015")
DEFAULT_POLICY = HERE / "selection-policy.json"
ARCHIVE_ROOT = "asset-accuracy-20261015"
CHUNK = 1024 * 1024


class EvidenceError(RuntimeError):
    pass


class ArchiveTooLarge(EvidenceError):
    pass


def canonical_json(value: object) -> bytes:
    return (json.dumps(value, ensure_ascii=False, indent=2, sort_keys=True) + "\n").encode("utf-8")


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def safe_relative(value: str) -> str:
    path = PurePosixPath(value)
    if (not value or path.is_absolute() or "\\" in value
            or any(part in ("", ".", "..") for part in value.split("/"))
            or any(ord(char) < 32 or ord(char) == 127 for char in value)):
        raise EvidenceError("Unsafe relative archive/source path rejected")
    return str(path)


@dataclasses.dataclass(frozen=True)
class FileRecord:
    path: str
    size: int
    mtime_ns: int
    ctime_ns: int
    device: int
    inode: int

    @classmethod
    def from_stat(cls, path: str, info: os.stat_result) -> "FileRecord":
        return cls(safe_relative(path), info.st_size, info.st_mtime_ns,
                   info.st_ctime_ns, info.st_dev, info.st_ino)


def exclusion(relative: str, mode: int) -> str | None:
    parts = PurePosixPath(relative).parts
    if stat.S_ISLNK(mode):
        return "symlink_not_followed"
    if not (stat.S_ISREG(mode) or stat.S_ISDIR(mode)):
        return "nonregular_not_archived"
    if relative == "paused-wip/source-wip.patch":
        return "potential_url_userinfo"
    if "__pycache__" in parts:
        return "python_bytecode_cache"
    if parts[-1].endswith((".o", ".so", ".dylib", ".dll", ".exe", ".a", ".rlib", ".rmeta")):
        return "compiled_binary_artifact"
    if parts[-1].endswith((".pyc", ".pyo")):
        return "python_bytecode"
    if any(part.startswith("retained-") for part in parts[:-1]) and parts[-1].startswith("oviraptor_lib-"):
        return "retained_native_executable"
    if parts[0] == "paused-wip" and (
            fnmatch.fnmatch(parts[-1], "*.tar") or fnmatch.fnmatch(parts[-1], "*.tar.*")
            or parts[-1].endswith((".tgz", ".zip", ".bundle"))):
        return "redundant_paused_source_archive"
    return None


def open_root(source: Path) -> int:
    # Open every component without symlinks; neither source root nor its parents
    # may turn an evidence read into a different filesystem authority.
    absolute = source.absolute()
    descriptor = os.open("/", os.O_RDONLY | os.O_DIRECTORY)
    try:
        for part in absolute.parts[1:]:
            following = os.open(part, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW,
                                dir_fd=descriptor)
            os.close(descriptor)
            descriptor = following
        return descriptor
    except BaseException:
        os.close(descriptor)
        raise


def scan_tree(source: Path) -> tuple[list[FileRecord], list[dict]]:
    selected: list[FileRecord] = []
    excluded: list[dict] = []

    def walk(descriptor: int, prefix: str) -> None:
        for name in sorted(os.listdir(descriptor)):
            relative = safe_relative(f"{prefix}/{name}" if prefix else name)
            info = os.stat(name, dir_fd=descriptor, follow_symlinks=False)
            reason = exclusion(relative, info.st_mode)
            if reason:
                entry = {"path": relative, "bytes": info.st_size, "reason": reason}
                if stat.S_ISREG(info.st_mode):
                    entry["sha256"] = file_digest(source, FileRecord.from_stat(relative, info))
                excluded.append(entry)
                continue
            if stat.S_ISDIR(info.st_mode):
                child = os.open(name, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW,
                                dir_fd=descriptor)
                try:
                    actual = os.fstat(child)
                    if (actual.st_ino, actual.st_dev) != (info.st_ino, info.st_dev):
                        raise EvidenceError("Directory changed while selecting evidence")
                    walk(child, relative)
                finally:
                    os.close(child)
            elif stat.S_ISREG(info.st_mode):
                record = FileRecord.from_stat(relative, info)
                with regular_file(source, record) as stream:
                    magic = stream.read(8)
                if magic.startswith((b"\x7fELF", b"MZ", b"!<arch>\n", b"\xfe\xed\xfa\xce", b"\xfe\xed\xfa\xcf", b"\xce\xfa\xed\xfe", b"\xcf\xfa\xed\xfe", b"\xca\xfe\xba\xbe")):
                    excluded.append({"path": relative, "bytes": info.st_size,
                                     "sha256": file_digest(source, record), "reason": "compiled_binary_magic"})
                else:
                    selected.append(record)

    descriptor = open_root(source)
    try:
        walk(descriptor, "")
    finally:
        os.close(descriptor)
    return sorted(selected, key=lambda row: row.path), excluded


@contextlib.contextmanager
def regular_file(source: Path, expected: FileRecord) -> Iterator[BinaryIO]:
    parent = open_root(source)
    descriptor: int | None = None
    try:
        parts = safe_relative(expected.path).split("/")
        for part in parts[:-1]:
            child = os.open(part, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=parent)
            os.close(parent)
            parent = child
        descriptor = os.open(parts[-1], os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK, dir_fd=parent)
        before = os.fstat(descriptor)
        if not stat.S_ISREG(before.st_mode) or FileRecord.from_stat(expected.path, before) != expected:
            raise EvidenceError("Evidence file changed before capture")
        with os.fdopen(descriptor, "rb") as stream:
            descriptor = None
            yield stream
            after = os.fstat(stream.fileno())
            if FileRecord.from_stat(expected.path, after) != expected:
                raise EvidenceError("Evidence file changed during capture")
    finally:
        if descriptor is not None:
            os.close(descriptor)
        os.close(parent)


def file_digest(source: Path, record: FileRecord) -> str:
    result = hashlib.sha256()
    with regular_file(source, record) as stream:
        for block in iter(lambda: stream.read(CHUNK), b""):
            result.update(block)
    return result.hexdigest()


# These expressions are detectors only. Never serialize their matches, values,
# excerpts or the source line. False positives require a hash-bound review.
PRIVACY_PATTERNS = {
    "private_key_block": re.compile(rb"-----BEGIN (?:RSA |EC |OPENSSH |DSA |ENCRYPTED )?PRIVATE KEY-----"),
    "aws_access_key_id": re.compile(rb"\b(?:AKIA|ASIA)[A-Z0-9]{16}\b"),
    "github_token_shape": re.compile(rb"\b(?:gh[pousr]_[A-Za-z0-9]{30,}|github_pat_[A-Za-z0-9_]{50,})\b"),
    "slack_token_shape": re.compile(rb"\bxox[baprs]-[A-Za-z0-9-]{20,}\b"),
    "api_token_shape": re.compile(rb"\bsk-(?:proj-)?[A-Za-z0-9_-]{24,}\b"),
    "bearer_token_shape": re.compile(rb"(?i)\bauthorization[\s\"']*[:=][\s\"']*bearer\s+[A-Za-z0-9._~+/-]{24,}"),
    "quoted_credential_assignment": re.compile(rb"(?i)[\"']?(?:api[_-]?key|access[_-]?token|password|secret[_-]?key)[\"']?\s*[:=]\s*[\"'][^\"'\r\n]{12,}[\"']"),
    "url_userinfo_shape": re.compile(rb"https?://[^\s/<>\"']{1,160}:[^\s/<>\"']{1,160}@"),
}


def privacy_scan(source: Path, selected: list[FileRecord], hashes: dict[str, str]) -> dict:
    candidates: list[dict] = []
    binary: list[str] = []
    for record in selected:
        with regular_file(source, record) as stream:
            prefix = stream.read(8192)
            if b"\0" in prefix or record.path.lower().endswith((".png", ".jpg", ".jpeg", ".webp", ".gif", ".woff", ".woff2")):
                binary.append(record.path)
                continue
            stream.seek(0)
            for line_number, line in enumerate(stream, 1):
                for category, expression in PRIVACY_PATTERNS.items():
                    if expression.search(line):
                        candidates.append({"path": record.path, "line": line_number,
                                           "category": category, "fileSha256": hashes[record.path]})
    return {"version": 1, "assessment": "candidates_found" if candidates else "no_pattern_candidates_found",
            "candidateCount": len(candidates), "candidates": candidates,
            "binaryFilesNotPixelReviewed": binary,
            "limitations": "Offline heuristic scan only. No matched values are recorded. No claim of secret absence or screenshot pixel review. Synthetic examples can match. Final selected evidence must have zero candidates; a review attachment cannot override a match."}


def review_privacy(report: dict, review_path: Path | None) -> dict:
    # The coordinator requires zero candidates in the final selected set. An
    # attached review documents context; it cannot override a positive detector.
    if report["candidateCount"]:
        raise EvidenceError(f"Privacy selection is not clear: {report['candidateCount']} candidate locations (no values emitted)")
    return {"candidateCount": 0, "reviewedCandidateCount": 0,
            "reviewFileSha256": digest(review_path.read_bytes()) if review_path else None,
            "pixelReviewClaim": "No pixel review performed by this packer; consult original local visual-review records."}


def group(relative: str) -> str:
    first = relative.split("/")[0]
    if "/" in relative:
        return first
    if relative.endswith((".py", ".sh")) or "policy" in relative or "environment" in relative:
        return "root-helpers-policy-environment"
    if any(word in relative for word in ("node", "frontend")):
        return "root-node-records"
    if any(word in relative for word in ("native", "targets", "suite", "cache")):
        return "root-native-records"
    return "root-review-and-other-records"


def index_markdown(selected: list[FileRecord], frozen_head: str) -> bytes:
    groups: dict[str, list[FileRecord]] = {}
    for record in selected:
        groups.setdefault(group(record.path), []).append(record)
    lines = ["# External asset accuracy: complete evidence archive", "",
             f"Frozen source label supplied by the coordinator: `{frozen_head}`.", "",
             "This archive preserves the original evidence, including failed attempts. Packaging runs no application tests and does not assert that compilation, test listing, or any historical run was a runtime pass. Original commands, exit receipts, declarations, selection ledgers and logs are the authority. No Git push is performed by this script.", "",
             "`asOf` or pack time does not make a historical result current. Browser IPC fixtures do not prove real Tauri/database/provider execution; native SQLite fixtures do not prove a user's database or Mac installation. Screenshots remain synthetic QA evidence.", "",
             "## Integrity and source selection", "",
             "From this extracted directory, run `sha256sum -c SHA256SUMS`. `manifest.json` records every original evidence file's SHA256/size/mtime and publication payload checksums. `SHA256SUMS` additionally covers the JSON manifest. The outer archive SHA256 is published beside the archive. Tar paths are relative, unique, and regular-file-only; no links are archived.", "",
             "`publication/selection-ledger.json` lists every included/excluded path and its explicit reason. Explicit exclusions cover retained test executables and compiled artifacts, Python caches, redundant paused source archives, nonregular/symlink objects, and the exact conservatively excluded paused-WIP source patch. The paused-WIP source patch is additionally excluded, unchanged in cloud storage, because its URL-userinfo candidates were not published; its path/size/hash/reason remain in the ledger. Old failed logs, their stderr/triage, frozen inputs and source-ready reviews remain included. Before/after input manifests inside the original evidence are preserved verbatim.", "",
             "`publication/privacy-scan.json` records candidate locations without any matched values. Automated scanning is heuristic and does not inspect PNG pixels. `publication/privacy-review.json`, when present, records the external review supplied for exact file hashes; no evidence file is silently redacted.", "",
             "## Records by area", "", "| Area | Files | Bytes |", "| --- | ---: | ---: |"]
    for name, records in sorted(groups.items()):
        lines.append(f"| {name} | {len(records)} | {sum(row.size for row in records)} |")
    lines += ["", "## Complete file index", ""]
    for record in selected:
        # Markdown label escapes cannot alter the actual verified tar path.
        label = record.path.replace("\\", "\\\\").replace("[", "\\[").replace("]", "\\]")
        lines.append(f"- [{label}](<evidence/{record.path}>) — {record.size} bytes")
    lines += ["", "## Publication helpers", "",
              "`publication/package_evidence.py` and `publication/selection-policy.json` are the exact packer and policy used. This pack does not contain retained native executables or redundant source bundles. Source remains available through the reviewable Git branch; the intended repository evidence destination is `docs/asset-accuracy-20261015`.", ""]
    return "\n".join(lines).encode("utf-8")


class HashingReader:
    def __init__(self, stream: BinaryIO):
        self.stream = stream
        self.hash = hashlib.sha256()
    def read(self, size: int = -1) -> bytes:
        data = self.stream.read(size)
        self.hash.update(data)
        return data


def add_bytes(archive: tarfile.TarFile, relative: str, content: bytes, archive_root: str = ARCHIVE_ROOT) -> None:
    info = tarfile.TarInfo(f"{archive_root}/{safe_relative(relative)}")
    info.size = len(content)
    info.mode = 0o644
    info.mtime = 0
    info.uid = info.gid = 0
    info.uname = info.gname = ""
    archive.addfile(info, io.BytesIO(content))


def verify_archive(path: Path, expected_sha256: str | None = None) -> dict:
    with path.open("rb") as stream:
        actual_archive_hash = hashlib.file_digest(stream, "sha256").hexdigest()
    if expected_sha256 and actual_archive_hash != expected_sha256:
        raise EvidenceError("Outer archive SHA256 mismatch")
    hashes: dict[str, str] = {}
    manifest = None
    found_root = None
    with tarfile.open(path, "r:gz") as archive:
        for member in archive:
            name = safe_relative(member.name)
            member_root, separator, relative = name.partition("/")
            if (not separator or not re.fullmatch(re.escape(ARCHIVE_ROOT) + r"(?:-[a-z0-9-]+)?", member_root)
                    or not member.isfile() or member.linkname):
                raise EvidenceError("Archive contains forbidden entry type/root/link")
            if found_root is not None and found_root != member_root:
                raise EvidenceError("Archive mixes independent roots")
            found_root = member_root
            if relative in hashes:
                raise EvidenceError("Duplicate archive member rejected")
            extracted = archive.extractfile(member)
            if extracted is None:
                raise EvidenceError("Archive member is unreadable")
            result = hashlib.sha256()
            if relative == "SHA256SUMS":
                raw = extracted.read()
                manifest = raw.decode("utf-8")
                result.update(raw)
            else:
                for block in iter(lambda: extracted.read(CHUNK), b""):
                    result.update(block)
            hashes[relative] = result.hexdigest()
    if manifest is None:
        raise EvidenceError("Internal SHA256SUMS is missing")
    expected = {}
    for line in manifest.splitlines():
        if not re.fullmatch(r"[a-f0-9]{64}  .+", line):
            raise EvidenceError("Malformed internal checksum line")
        checksum, relative = line.split("  ", 1)
        safe_relative(relative)
        if relative in expected:
            raise EvidenceError("Duplicate checksum entry")
        expected[relative] = checksum
    if expected != {name: value for name, value in hashes.items() if name != "SHA256SUMS"}:
        raise EvidenceError("Internal checksum set or content mismatch")
    return {"operation": "archive_integrity_verification", "sha256": actual_archive_hash,
            "bytes": path.stat().st_size, "verifiedRegularFiles": len(hashes),
            "applicationTestsExecuted": False, "gitPushPerformed": False}


def capture(source: Path, policy: dict) -> tuple[list[FileRecord], list[dict], dict[str, str], dict]:
    selected, excluded = scan_tree(source)
    if not selected:
        raise EvidenceError("Evidence selection is empty")
    maximum = policy["limits"]["maximumIndividualEvidenceBytes"]
    if any(record.size > maximum for record in selected):
        raise EvidenceError("Selected evidence exceeds per-file limit; review policy instead of silently omitting it")
    hashes = {record.path: file_digest(source, record) for record in selected}
    privacy = privacy_scan(source, selected, hashes)
    current, _ = scan_tree(source)
    if current != selected:
        raise EvidenceError("Evidence is still changing; capture is provisional and must be retried after writers stop")
    return selected, excluded, hashes, privacy


def pack_one(source: Path, output: Path, frozen_head: str, policy_path: Path,
             review_path: Path | None = None, part: str = "all",
             selected_paths: set[str] | None = None) -> dict:
    policy_bytes = policy_path.read_bytes()
    policy = json.loads(policy_bytes)
    if not re.fullmatch(r"[a-f0-9]{40}", frozen_head):
        raise EvidenceError("Final pack requires the full coordinator-supplied frozen Git SHA")
    if os.path.lexists(output):
        raise EvidenceError("Output must be a new directory; old evidence is never overwritten")
    source_absolute, output_absolute = source.absolute(), output.absolute()
    if output_absolute == source_absolute or source_absolute in output_absolute.parents:
        raise EvidenceError("Publication output cannot be inside the captured evidence tree")
    all_selected, excluded, hashes, privacy = capture(source, policy)
    selected = all_selected if selected_paths is None else [row for row in all_selected if row.path in selected_paths]
    if not selected or selected_paths is not None and set(row.path for row in selected) != selected_paths:
        raise EvidenceError("Publication part has an empty or changed source selection")
    excluded = excluded + [{"path": row.path, "bytes": row.size, "reason": "published_in_another_archive_part"} for row in all_selected if row not in selected]
    archive_root = ARCHIVE_ROOT if part == "all" else f"{ARCHIVE_ROOT}-{part}"
    archive_basename = "evidence" if part == "all" else part
    assessment = review_privacy(privacy, review_path)
    ledger = {"version": 1, "sourceRoot": str(source_absolute), "selectedCount": len(selected),
              "selectedBytes": sum(record.size for record in selected),
              "excluded": excluded, "selectionPolicySha256": digest(policy_bytes),
              "selected": [dataclasses.asdict(record) for record in selected]}
    payload = {
        "INDEX.md": (f"Publication part: {part}. Other selected evidence may be in companion archives.\n\n".encode() + index_markdown(selected, frozen_head)),
        "publication/package_evidence.py": Path(__file__).read_bytes(),
        "publication/selection-policy.json": policy_bytes,
        "publication/selection-ledger.json": canonical_json(ledger),
        "publication/privacy-scan.json": canonical_json(privacy),
        "publication/privacy-assessment.json": canonical_json(assessment),
        "publication/capture-scope.json": canonical_json({"frozenSourceLabel": frozen_head,
            "repositoryTarget": policy["repositoryTarget"], "sourceSelectionStable": True,
            "applicationTestsExecuted": False, "gitPushPerformed": False,
            "testResultAuthority": "Original unmodified attempt records, not this packer",
            "inputManifests": "Original per-run before/after input manifests preserved under evidence/"}),
    }
    for helper in ("README.md", "test_package_evidence.py", "self-tests-final.json", "self-tests-final.log", "privacy-01.json", "privacy-02-failed.json", "privacy-02-failed.log"):
        local = HERE / helper
        if local.is_file() and not local.is_symlink():
            payload[f"publication/{helper}"] = local.read_bytes()
    if review_path:
        payload["publication/privacy-review.json"] = review_path.read_bytes()
    for name, value in payload.items():
        if any(expression.search(value) for expression in PRIVACY_PATTERNS.values()):
            raise EvidenceError("Generated publication payload contains a privacy candidate; no values emitted")
    entries = [{"path": f"evidence/{record.path}", "bytes": record.size,
                "sha256": hashes[record.path], "sourceMtimeNs": record.mtime_ns}
               for record in selected]
    entries += [{"path": name, "bytes": len(value), "sha256": digest(value)}
                for name, value in sorted(payload.items())]
    payload["manifest.json"] = canonical_json({"version": 1, "archiveRoot": archive_root,
        "publicationPart": part, "frozenSourceLabel": frozen_head, "files": entries,
        "checksumScope": "All evidence/publication payloads; SHA256SUMS additionally covers this manifest. The outer archive hash covers SHA256SUMS itself."})
    checksums = {entry["path"]: entry["sha256"] for entry in entries}
    checksums["manifest.json"] = digest(payload["manifest.json"])
    payload["SHA256SUMS"] = "".join(f"{value}  {name}\n" for name, value in sorted(checksums.items())).encode("utf-8")
    output.parent.mkdir(parents=True, exist_ok=True)
    temporary = Path(tempfile.mkdtemp(prefix=".evidence-pack-", dir=output.parent))
    try:
        archive_path = temporary / f"{archive_basename}.tar.gz"
        with archive_path.open("xb") as raw:
            with gzip.GzipFile(filename="", mode="wb", fileobj=raw, mtime=0) as compressed:
                with tarfile.open(fileobj=compressed, mode="w|", format=tarfile.PAX_FORMAT) as archive:
                    for record in selected:
                        info = tarfile.TarInfo(f"{archive_root}/evidence/{record.path}")
                        info.size, info.mode, info.mtime = record.size, 0o644, 0
                        with regular_file(source, record) as stream:
                            reader = HashingReader(stream)
                            archive.addfile(info, reader)
                            if reader.hash.hexdigest() != hashes[record.path]:
                                raise EvidenceError("Evidence bytes changed between hash and archive capture")
                    for name, value in sorted(payload.items()):
                        add_bytes(archive, name, value, archive_root)
        if archive_path.stat().st_size > policy["limits"]["maximumArchiveBytes"]:
            raise ArchiveTooLarge("Archive exceeds per-part publication limit")
        after, _ = scan_tree(source)
        if after != all_selected:
            raise EvidenceError("Evidence tree changed during pack; no final archive published")
        verified = verify_archive(archive_path)
        receipt = {**verified, "operation": "evidence_archive_created", "frozenSourceLabel": frozen_head,
                   "selectedSourceFiles": len(selected), "selectedSourceBytes": sum(row.size for row in selected),
                   "excludedEntries": len(excluded), "privacyAssessment": assessment,
                   "publicationPart": part, "archiveRoot": archive_root,
                   "claim": "Archive integrity only; original evidence retains all pass/fail/list/build meaning."}
        (temporary / f"{archive_basename}.tar.gz.sha256").write_text(f"{verified['sha256']}  {archive_basename}.tar.gz\n", "utf-8")
        (temporary / "publication-receipt.json").write_bytes(canonical_json(receipt))
        (temporary / "INDEX.md").write_bytes(payload["INDEX.md"])
        os.rename(temporary, output)
        return receipt
    finally:
        if temporary.exists():
            shutil.rmtree(temporary)



def is_screenshot(relative: str) -> bool:
    return relative.lower().endswith((".png", ".jpg", ".jpeg", ".webp", ".gif"))


def pack(source: Path, output: Path, frozen_head: str, policy_path: Path,
         review_path: Path | None = None) -> dict:
    """Prefer one archive; split text/screenshots and then bounded parts if needed.

    Each part has its own root, index, manifest, checksum set and privacy/policy
    provenance. No files disappear because a compressed size limit was exceeded.
    """
    if os.path.lexists(output):
        raise EvidenceError("Output must be a new directory; old evidence is never overwritten")
    if output.absolute() == source.absolute() or source.absolute() in output.absolute().parents:
        raise EvidenceError("Publication output cannot be inside the captured evidence tree")
    output.parent.mkdir(parents=True, exist_ok=True)
    staging = Path(tempfile.mkdtemp(prefix=".publication-set-", dir=output.parent))
    selected, _ = scan_tree(source)
    original = selected
    receipts: list[dict] = []
    assignments: dict[str, set[str]] = {}
    try:
        try:
            one = pack_one(source, staging / "combined", frozen_head, policy_path, review_path)
            receipts.append(one)
            assignments["all"] = {row.path for row in selected}
        except ArchiveTooLarge:
            for category in ("texts", "screenshots"):
                rows = [row for row in selected if is_screenshot(row.path) == (category == "screenshots")]
                if not rows:
                    continue
                paths = {row.path for row in rows}
                try:
                    one = pack_one(source, staging / category, frozen_head, policy_path, review_path, category, paths)
                    receipts.append(one)
                    assignments[category] = paths
                except ArchiveTooLarge:
                    chunks: list[list[FileRecord]] = []
                    budget = min(18 * 1024 * 1024, json.loads(policy_path.read_text())["limits"]["maximumArchiveBytes"] * 9 // 10)
                    current: list[FileRecord] = []
                    size = 0
                    for row in rows:
                        if row.size > budget:
                            raise EvidenceError("One evidence file exceeds safe split budget; preserve it and request an explicit publication strategy")
                        if current and size + row.size > budget:
                            chunks.append(current)
                            current, size = [], 0
                        current.append(row)
                        size += row.size
                    if current:
                        chunks.append(current)
                    for number, chunk in enumerate(chunks, 1):
                        part = f"{category}-{number:02d}"
                        paths = {row.path for row in chunk}
                        one = pack_one(source, staging / part, frozen_head, policy_path, review_path, part, paths)
                        receipts.append(one)
                        assignments[part] = paths
        after, _ = scan_tree(source)
        if after != original:
            raise EvidenceError("Evidence changed between companion archive captures")
        flattened: set[str] = set()
        for paths in assignments.values():
            if flattened.intersection(paths):
                raise EvidenceError("Source evidence duplicated across publication parts")
            flattened.update(paths)
        if flattened != {row.path for row in original}:
            raise EvidenceError("Publication parts omit selected source evidence")
        published = staging / "ready"
        published.mkdir()
        archives = []
        index = ["# Asset accuracy evidence publication set", "",
                 f"Coordinator-supplied frozen source: `{frozen_head}`.", "",
                 "This is an archive-integrity index, not an application test result. All original failure, build/list, runtime, policy, input and review records retain their original meaning. No tests or Git pushes were performed by the packer.", "",
                 "Companion archives use independent internal roots; extracting all of them cannot overwrite another part's manifest. Every archive has its own INDEX.md/manifest.json/SHA256SUMS and exact packer/policy. Text evidence includes HTML/DOM snapshots; image screenshots are separated only when needed for the 20 MiB archive cap.", "",
                 "| Archive | Bytes | SHA256 | Source files |", "| --- | ---: | --- | ---: |"]
        for receipt in receipts:
            part = receipt["publicationPart"]
            directory = staging / ("combined" if part == "all" else part)
            basename = "evidence" if part == "all" else part
            filename = f"{basename}.tar.gz"
            shutil.move(str(directory / filename), published / filename)
            shutil.move(str(directory / f"{filename}.sha256"), published / f"{filename}.sha256")
            shutil.move(str(directory / "publication-receipt.json"), published / f"{basename}-receipt.json")
            shutil.move(str(directory / "INDEX.md"), published / f"{basename}-INDEX.md")
            archives.append({"file": filename, **receipt})
            index.append(f"| [{filename}]({filename}) | {receipt['bytes']} | `{receipt['sha256']}` | {receipt['selectedSourceFiles']} |")
        result = {"operation": "evidence_publication_set_created", "frozenSourceLabel": frozen_head,
                  "selectedSourceFiles": len(original), "selectedSourceBytes": sum(row.size for row in original),
                  "archives": archives, "allSelectedEvidenceCoveredExactlyOnce": True,
                  "applicationTestsExecuted": False, "gitPushPerformed": False}
        (published / "PUBLICATION-INDEX.md").write_text("\n".join(index) + "\n", "utf-8")
        (published / "publication-set.json").write_bytes(canonical_json(result))
        os.rename(published, output)
        return result
    finally:
        shutil.rmtree(staging)


def write_new(path: Path, value: object) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("xb") as stream:
        stream.write(canonical_json(value))


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("mode", choices=("plan", "privacy", "pack", "verify"))
    parser.add_argument("--source", type=Path, default=DEFAULT_SOURCE)
    parser.add_argument("--policy", type=Path, default=DEFAULT_POLICY)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--frozen-head")
    parser.add_argument("--privacy-review", type=Path)
    parser.add_argument("--validation-complete", action="store_true")
    parser.add_argument("--expected-sha256")
    args = parser.parse_args()
    try:
        if args.mode == "verify":
            result = verify_archive(args.output, args.expected_sha256)
        elif args.mode == "pack":
            if not args.validation_complete:
                raise EvidenceError("Final pack is disabled until the coordinator confirms validation writers have finished")
            result = pack(args.source, args.output, args.frozen_head or "", args.policy, args.privacy_review)
        else:
            selected, excluded = scan_tree(args.source)
            result = {"operation": f"provisional_{args.mode}", "selectedFiles": len(selected),
                      "selectedBytes": sum(row.size for row in selected), "excluded": excluded,
                      "applicationTestsExecuted": False, "finalArchiveCreated": False}
            if args.mode == "privacy":
                hashes = {row.path: file_digest(args.source, row) for row in selected}
                result["privacy"] = privacy_scan(args.source, selected, hashes)
            else:
                result["selected"] = [dataclasses.asdict(row) for row in selected]
            write_new(args.output, result)
        # Never print matched values, original lines or inferred runtime outcomes.
        summary = {key: value for key, value in result.items()
                   if key in {"operation", "selectedFiles", "selectedBytes", "sha256", "bytes", "verifiedRegularFiles", "selectedSourceFiles", "applicationTestsExecuted", "finalArchiveCreated", "gitPushPerformed"}}
        if "privacy" in result:
            summary["privacyCandidatesFound"] = result["privacy"]["candidateCount"] > 0
            summary["privacyCandidateCount"] = result["privacy"]["candidateCount"]
        print(json.dumps(summary, ensure_ascii=False, sort_keys=True))
    except (EvidenceError, OSError, ValueError, tarfile.TarError) as error:
        # Our own messages are value-free; OS/JSON errors may contain raw names,
        # so expose only their class without secret-bearing exception details.
        detail = str(error) if isinstance(error, EvidenceError) else type(error).__name__
        raise SystemExit(f"Evidence operation failed: {detail}") from None


if __name__ == "__main__":
    main()
