pub mod set_account;
pub mod set_token_account;
pub mod reset_account;
pub mod stream_account;

pub trait CheatcodeBuilder {
    const METHOD: &'static str;
    fn build(self) -> serde_json::Value;
}
