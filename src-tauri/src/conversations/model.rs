use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Conversation {
    pub id: String,
    pub project_id: i64,
    pub title: String,
    pub title_source: String,
    pub archived: bool,
    pub revision: i64,
    pub message_count: i64,
    pub last_message_preview: String,
    pub created_at: String,
    pub updated_at: String,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Message {
    pub id: String,
    pub conversation_id: String,
    pub sequence: i64,
    pub role: String,
    pub content: String,
    pub reply_to_message_id: Option<String>,
    pub created_at: String,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Link {
    pub kind: String,
    pub target_id: String,
    pub label: String,
    pub available: bool,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplyAvailability {
    pub available: bool,
    pub code: String,
    pub message: String,
    pub model: Option<String>,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Reply {
    pub id: String,
    pub user_message_id: String,
    pub status: String,
    pub code: String,
    pub message: String,
    pub assistant_message_id: Option<String>,
    pub outcome_unknown: bool,
    pub created_at: String,
    pub updated_at: String,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationPage {
    pub items: Vec<Conversation>,
    pub total: i64,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationView {
    pub conversation: Conversation,
    pub messages: Vec<Message>,
    pub has_more: bool,
    pub links: Vec<Link>,
    pub reply_availability: ReplyAvailability,
    pub latest_reply: Option<Reply>,
    pub tool_activity: Vec<ToolActivity>,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolActivity {
    pub id: String,
    pub reply_id: String,
    pub user_message_id: String,
    pub round: i64,
    pub call_id: String,
    pub name: String,
    pub status: String,
    pub summary: String,
    pub arguments: serde_json::Value,
    pub created_at: String,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SendReceipt {
    pub conversation: Conversation,
    pub user_message: Message,
    pub reply: Option<Reply>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ListInput {
    pub project_id: i64,
    pub query: Option<String>,
    pub include_archived: Option<bool>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateInput {
    pub project_id: i64,
    pub operation_id: String,
    pub title: Option<String>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReadInput {
    pub project_id: i64,
    pub conversation_id: String,
    pub before_sequence: Option<i64>,
    pub limit: Option<i64>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UpdateInput {
    pub project_id: i64,
    pub conversation_id: String,
    pub expected_revision: i64,
    pub title: Option<String>,
    pub archived: Option<bool>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LinkInput {
    pub kind: String,
    pub target_id: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SetLinksInput {
    pub project_id: i64,
    pub conversation_id: String,
    pub expected_revision: i64,
    pub links: Vec<LinkInput>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SendInput {
    pub project_id: i64,
    pub conversation_id: String,
    pub expected_revision: i64,
    pub operation_id: String,
    pub content: String,
    pub request_reply: bool,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CancelInput {
    pub project_id: i64,
    pub conversation_id: String,
    pub reply_id: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeleteInput {
    pub project_id: i64,
    pub conversation_id: String,
    pub expected_revision: i64,
}
