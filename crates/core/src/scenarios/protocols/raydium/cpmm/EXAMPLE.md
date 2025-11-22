# Raydium CPMM Scenario Example

This example demonstrates how to use the Raydium CPMM PoolState template to override pool liquidity, fees, and status for testing AMM scenarios.

## Verification Steps (For Maintainers)

To verify the template works without executing trades:

1. **Register a Test Scenario**:
   Use an arbitrary Pubkey (e.g., `8888...`) as the target pool account.

   ```javascript
   const scenario = {
     id: "test-raydium-pool-override",
     name: "Test Pool State Override",
     overrides: [
       {
         template_id: "raydium-cpmm-pool-state-override",
         values: {
           "protocol_fees_token_0": 5000000,
           "protocol_fees_token_1": 3000000,
           "fund_fees_token_0": 2000000,
           "fund_fees_token_1": 1500000,
           "lp_supply": 999999999,
           "status": 4, // Disable swap (bit2 = 1)
           "open_time": 1700000000
         },
         "account": {
           "Pubkey": "88888888888888888888888888888888888888888888"
         }
       }
     ]
   };
   await connection.send("surfnet_registerScenario", [scenario]);
   ```

2. **Verify On-Chain State**:
   Fetch the account info for `88888888888888888888888888888888888888888888`.

   ```javascript
   const accountInfo = await connection.getAccountInfo(
     new PublicKey("88888888888888888888888888888888888888888888")
   );
   
   // Verify Discriminator (First 8 bytes)
   // Should be: [247, 237, 227, 245, 215, 195, 222, 70]
   console.log(Array.from(accountInfo.data.slice(0, 8)));

   // Verify protocol_fees_token_0 (at byte offset after pubkeys)
   // Confirm values match what you set
   ```

## Use Case

Test edge cases in Raydium CPMM pools by manipulating pool state:
- **Liquidity Testing**: Simulate low/high liquidity scenarios
- **Fee Accumulation**: Test protocol and fund fee collection logic
- **Pool Status**: Verify error handling when swaps/deposits/withdraws are disabled
- **Time-based Conditions**: Test behavior before/after pool opening time

## Template Details

**Template ID**: `raydium-cpmm-pool-state-override`

**Overridable Properties**:
- `amm_config` (Pubkey) - Config account address
- `pool_creator` (Pubkey) - Pool creator address
- `token_0_vault` (Pubkey) - Token A vault address
- `token_1_vault` (Pubkey) - Token B vault address
- `lp_mint` (Pubkey) - LP token mint address
- `token_0_mint` (Pubkey) - Token A mint address
- `token_1_mint` (Pubkey) - Token B mint address
- `token_0_program` (Pubkey) - Token A program ID
- `token_1_program` (Pubkey) - Token B program ID
- `observation_key` (Pubkey) - Observation account address
- `auth_bump` (u8) - Authority bump seed
- `status` (u8) - Pool status bits (0=normal, 1=disable deposit, 2=disable withdraw, 4=disable swap)
- `lp_mint_decimals` (u8) - LP token decimals
- `mint_0_decimals` (u8) - Token A decimals
- `mint_1_decimals` (u8) - Token B decimals
- `lp_supply` (u64) - Total LP token supply in circulation
- `protocol_fees_token_0` (u64) - Accumulated protocol fees for token A
- `protocol_fees_token_1` (u64) - Accumulated protocol fees for token B
- `fund_fees_token_0` (u64) - Accumulated fund fees for token A
- `fund_fees_token_1` (u64) - Accumulated fund fees for token B
- `open_time` (u64) - Unix timestamp when trading is allowed
- `recent_epoch` (u64) - Recent epoch number

## Example Scenarios

### Scenario 1: Simulate Low Liquidity Pool
Test slippage behavior with minimal LP supply:
```json
{
  "values": {
    "lp_supply": 1000
  }
}
```

### Scenario 2: Test Fee Collection
Set high accumulated fees to verify collection logic:
```json
{
  "values": {
    "protocol_fees_token_0": 10000000000,
    "protocol_fees_token_1": 8000000000,
    "fund_fees_token_0": 5000000000,
    "fund_fees_token_1": 4000000000
  }
}
```

### Scenario 3: Disable Swaps
Force swap transactions to fail for error handling tests:
```json
{
  "values": {
    "status": 4
  }
}
```

### Scenario 4: Pool Not Yet Open
Test behavior before pool launch:
```json
{
  "values": {
    "open_time": 9999999999
  }
}
```

## Integration with Surfpool Studio

This template will automatically appear in Surfpool Studio's drag-and-drop UI under:
- **Protocol**: Raydium
- **Tags**: dex, amm, liquidity, defi

Simply drag the "Override Raydium CPMM Pool State" template into your scenario timeline and configure the pool properties through the UI.

## Notes

- PoolState accounts are persistent and exist for the lifetime of the pool
- You must specify the exact PoolState account address (can be found via Raydium's API or explorers)
- The `fetch_before_use` option can refresh pool data from mainnet before applying overrides
- Multiple overrides can be scheduled at different slots to test dynamic pool conditions
