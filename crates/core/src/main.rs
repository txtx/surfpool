
#[macro_use]
extern crate log;

#[macro_use]
extern crate serde_derive;

#[cfg(test)]
#[macro_use]
extern crate serde_json;

mod runloop;
mod rpc;

use litesvm::LiteSVM;

fn main() {
    println!("Where you train before surfing Solana");
    let mut svm = LiteSVM::new();
    // let payer_kp = Keypair::new();
    // let payer_pk = payer_kp.pubkey();
    // let program_id = Pubkey::new_unique();

    // svm.warp_to_slot(1);
    // svm.add_program(program_id, &read_counter_program());
    // svm.airdrop(&payer_pk, 1000000000).unwrap();
    // let rent: Rent = svm.get_sysvar();
    // println!("{:?}", rent);
    // let clock: Clock = svm.get_sysvar();
    // println!("{:?}", clock);
    // let blockhash = svm.latest_blockhash();
    // println!("{:?}", blockhash);
    // svm.warp_to_slot(2);
    // let clock: Clock = svm.get_sysvar();
    // println!("{:?}", clock);
    // svm.expire_blockhash();
    // let blockhash = svm.latest_blockhash();
    // println!("{:?}", blockhash);

    // let sns_program_id = Pubkey::from_str("58PwtjSDuFHuUkYjH9BYnnQKHfwo9reZhC2zMJv9JPkx").unwrap();
    // println!("{:?}", sns_program_id);

    // // let mainnet_program_id = svm.get
    // let account = svm.get_account(&sns_program_id);
    // println!("{:?}", account);

    // // solana_rpc_client::rpc_client::RpcClient::new(url)
    // let rpc_client = RpcClient::new("https://api.mainnet-beta.solana.com");
    // let mainnet_account = rpc_client.get_account(&sns_program_id).unwrap();
    // println!("{:?}", mainnet_account);

    // let _ = svm.set_account(sns_program_id, mainnet_account).unwrap();

    // let account = svm.get_account(&sns_program_id);
    // println!("{:?}", account);

    hiro_system_kit::nestable_block_on(runloop::start(&mut svm));
}
