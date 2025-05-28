use serde::Serialize;

#[derive(Serialize)]
pub struct SetTokenAccountResponse {
    success: AccountUpdated,
    error: Option<String>,
}

#[derive(Serialize)]
pub struct AccountUpdated {
    account: SeededAccount,
    token_address: String,
    token_symbol: Option<String>,
    token_account_address: String,
    amount: u64,
}

#[derive(Serialize)]
pub enum SeededAccount {
    Provided(String),
    Generated(NewAccount),
}

#[derive(Serialize)]
pub struct NewAccount {
    secret_key: String,
    public_key: String,
}

pub fn run(
    surfnet_address: String,
    wallet_address: Option<String>,
    token: String,
    token_amount: Option<u64>,
) -> SetTokenAccountResponse {
    todo!("send an RPC request `surfnet_setTokenAccount` with the right parameters")
}
