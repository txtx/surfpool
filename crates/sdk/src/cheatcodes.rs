use solana_pubkey::Pubkey;
use solana_client::rpc_request::RpcRequest;
use solana_rpc_client::rpc_client::RpcClient;
use spl_associated_token_account_interface::address::get_associated_token_address_with_program_id;

use crate::error::{SurfnetError, SurfnetResult};

/// Direct state manipulation helpers for a running Surfnet.
///
/// These bypass normal transaction flow to instantly set account state —
/// perfect for test setup (funding wallets, minting tokens, etc.).
///
/// ```rust,no_run
/// use surfpool_sdk::{Surfnet, Pubkey};
///
/// # async fn example() {
/// let surfnet = Surfnet::start().await.unwrap();
/// let cheats = surfnet.cheatcodes();
///
/// // Fund an account with 5 SOL
/// let alice: Pubkey = "...".parse().unwrap();
/// cheats.fund_sol(&alice, 5_000_000_000).unwrap();
///
/// // Fund a token account with 1000 USDC
/// let usdc_mint: Pubkey = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v".parse().unwrap();
/// cheats.fund_token(&alice, &usdc_mint, 1_000_000_000, None).unwrap();
/// # }
/// ```
pub struct Cheatcodes<'a> {
    rpc_url: &'a str,
}

impl<'a> Cheatcodes<'a> {
    pub(crate) fn new(rpc_url: &'a str) -> Self {
        Self { rpc_url }
    }

    fn rpc_client(&self) -> RpcClient {
        RpcClient::new(self.rpc_url)
    }

    /// Set the SOL balance (in lamports) for an account.
    pub fn fund_sol(&self, address: &Pubkey, lamports: u64) -> SurfnetResult<()> {
        let params = serde_json::json!([
            address.to_string(),
            { "lamports": lamports }
        ]);
        self.call_cheatcode("surfnet_setAccount", params)
    }

    /// Set arbitrary account data.
    pub fn set_account(
        &self,
        address: &Pubkey,
        lamports: u64,
        data: &[u8],
        owner: &Pubkey,
    ) -> SurfnetResult<()> {
        let params = serde_json::json!([
            address.to_string(),
            {
                "lamports": lamports,
                "data": data,
                "owner": owner.to_string()
            }
        ]);
        self.call_cheatcode("surfnet_setAccount", params)
    }

    /// Fund a token account (creates the ATA if needed).
    ///
    /// Uses `spl_token` program by default. Pass `token_program` to use Token-2022.
    pub fn fund_token(
        &self,
        owner: &Pubkey,
        mint: &Pubkey,
        amount: u64,
        token_program: Option<&Pubkey>,
    ) -> SurfnetResult<()> {
        let program = token_program.copied().unwrap_or(spl_token_program_id());
        let params = serde_json::json!([
            owner.to_string(),
            mint.to_string(),
            { "amount": amount },
            program.to_string()
        ]);
        self.call_cheatcode("surfnet_setTokenAccount", params)
    }

    /// Set a token account balance for a specific ATA address.
    pub fn set_token_balance(
        &self,
        owner: &Pubkey,
        mint: &Pubkey,
        amount: u64,
        token_program: Option<&Pubkey>,
    ) -> SurfnetResult<()> {
        self.fund_token(owner, mint, amount, token_program)
    }

    /// Get the associated token address for a wallet/mint pair.
    pub fn get_ata(&self, owner: &Pubkey, mint: &Pubkey, token_program: Option<&Pubkey>) -> Pubkey {
        let program = token_program.copied().unwrap_or(spl_token_program_id());
        get_associated_token_address_with_program_id(owner, mint, &program)
    }

    /// Fund multiple accounts with SOL in one call.
    pub fn fund_sol_many(&self, accounts: &[(&Pubkey, u64)]) -> SurfnetResult<()> {
        for (address, lamports) in accounts {
            self.fund_sol(address, *lamports)?;
        }
        Ok(())
    }

    /// Fund multiple wallets with the same token and amount.
    pub fn fund_token_many(
        &self,
        owners: &[&Pubkey],
        mint: &Pubkey,
        amount: u64,
        token_program: Option<&Pubkey>,
    ) -> SurfnetResult<()> {
        for owner in owners {
            self.fund_token(owner, mint, amount, token_program)?;
        }
        Ok(())
    }

    fn call_cheatcode(
        &self,
        method: &'static str,
        params: serde_json::Value,
    ) -> SurfnetResult<()> {
        let client = self.rpc_client();
        client
            .send::<serde_json::Value>(
                RpcRequest::Custom { method },
                params,
            )
            .map_err(|e| SurfnetError::Cheatcode(format!("{method}: {e}")))?;
        Ok(())
    }
}

fn spl_token_program_id() -> Pubkey {
    // spl_token::id() = TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA
    Pubkey::from_str_const("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA")
}
