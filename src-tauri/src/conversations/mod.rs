//! Project-scoped human conversations. No scan, agent run, directive or tool
//! authority is created here. Only explicit Send may make one model request.
mod model;
mod reply;
mod store;
use crate::AppState;
pub use model::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use tauri::State;

// This is a process-local UI admission limit, not a cross-process budget. It
// bounds blocking reply workers before they spawn or persist a user message.
static REPLIES: AtomicUsize = AtomicUsize::new(0);
struct ReplyPermit;
impl ReplyPermit {
    fn acquire() -> Result<Self, String> {
        let mut count = REPLIES.load(Ordering::Acquire);
        loop {
            if count >= 2 {
                return Err("conversation_reply_capacity".into());
            }
            match REPLIES.compare_exchange_weak(
                count,
                count + 1,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return Ok(Self),
                Err(current) => count = current,
            }
        }
    }
}
impl Drop for ReplyPermit {
    fn drop(&mut self) {
        REPLIES.fetch_sub(1, Ordering::AcqRel);
    }
}
macro_rules! command {
    ($name:ident,$input:ty,$output:ty,$function:path) => {
        #[tauri::command]
        pub async fn $name(state: State<'_, AppState>, input: $input) -> Result<$output, String> {
            let path = state.db_path.clone();
            tauri::async_runtime::spawn_blocking(move || $function(&path, input))
                .await
                .map_err(|_| "conversation_worker_failed".to_string())?
        }
    };
}
command!(list_conversations, ListInput, ConversationPage, store::list);
command!(
    create_conversation,
    CreateInput,
    Conversation,
    store::create
);
command!(read_conversation, ReadInput, ConversationView, store::read);
command!(
    update_conversation,
    UpdateInput,
    Conversation,
    store::update
);
command!(
    set_conversation_links,
    SetLinksInput,
    Conversation,
    store::set_links
);
command!(cancel_conversation_reply, CancelInput, Reply, reply::cancel);
command!(delete_conversation, DeleteInput, (), store::delete);
#[tauri::command]
pub async fn send_conversation_message(
    state: State<'_, AppState>,
    input: SendInput,
) -> Result<SendReceipt, String> {
    let permit = if input.request_reply {
        Some(ReplyPermit::acquire()?)
    } else {
        None
    };
    let path = state.db_path.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _permit = permit;
        reply::send(&path, input)
    })
    .await
    .map_err(|_| "conversation_worker_failed".to_string())?
}
#[cfg(test)]
mod tests;
