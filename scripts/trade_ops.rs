use serde_json::Value;
use std::fs;
use std::thread::sleep;
use std::time::Duration;
use std::time::Instant;
use tokio::sync::mpsc;
use web3::transports::Http;
use web3::types::{Block, TransactionReceipt, TransactionRequest, U256};
use web3::Web3;

#[tokio::main]
async fn main() {
    let config = load_config("aurora").unwrap(); // Replace with your network name
    println!("Loaded {} routes", config["routes"].as_array().unwrap().len());

    let (sender, receiver) = mpsc::channel();
    let http = Http::new("YOUR_ETHEREUM_NODE_URL_HERE").unwrap(); // Replace with your Ethereum node URL
    let web3 = Web3::new(http);

    // Start the setup process
    let start_time = Instant::now();
    let (balances, arb) = setup(&config, &web3).await;
    let setup_time = start_time.elapsed().as_secs();
    println!("Setup time: {} seconds", setup_time);

    // Periodically log results
    let receiver_clone = receiver.clone();
    tokio::spawn(async move {
        loop {
            log_results(&receiver_clone, &balances, &web3).await;
            sleep(Duration::from_secs(600)); // Log results every 10 minutes
        }
    });

    // Start looking for dual trade opportunities
    look_for_dual_trade(config, arb, balances, sender, web3).await;
}

fn load_config(network: &str) -> Result<Value, Box<dyn std::error::Error>> {
    let config_path = format!("../config/{}.json", network);
    let config_str = fs::read_to_string(config_path)?;
    let config: Value = serde_json::from_str(&config_str)?;
    Ok(config)
}

async fn setup(
    config: &Value,
    web3: &Web3<Http>,
) -> (Vec<BalanceInfo>, web3::transports::ipc::Ipc) {
    let owner = "YOUR_OWNER_ADDRESS_HERE"; // Replace with your owner address
    let arb_contract_address = config["arbContract"]
        .as_str()
        .expect("Arb contract address not found");
    let arb_contract = web3
        .eth()
        .contract(
            include_bytes!("YOUR_ARB_CONTRACT_ABI_HERE"), // Replace with your Arb contract ABI
            arb_contract_address,
        );

    let mut balances = Vec::new();
    for base_asset in config["baseAssets"].as_array().unwrap() {
        let asset_address = base_asset["address"]
            .as_str()
            .expect("Asset address not found");
        let asset_contract = web3
            .eth()
            .contract(
                include_bytes!("YOUR_ASSET_CONTRACT_ABI_HERE"), // Replace with your asset contract ABI
                asset_address,
            );
        let balance: U256 = asset_contract
            .query("balanceOf", (), None, web3::transports::None)
            .await
            .expect("Failed to query balance");
        println!("{}: {}", base_asset["sym"], balance);
        balances.push(BalanceInfo {
            sym: base_asset["sym"].to_string(),
            balance,
            start_balance: balance,
        });
    }

    // Schedule periodic logging
    tokio::spawn(async move {
        loop {
            sleep(Duration::from_secs(600)); // Log results every 10 minutes
            log_results(&web3, &balances).await;
        }
    });

    (balances, arb_contract)
}

async fn log_results(web3: &Web3<Http>, balances: &[BalanceInfo]) {
    println!("############# LOGS #############");
    for (i, base_asset) in balances.iter().enumerate() {
        let asset_address = config["baseAssets"][i]["address"]
            .as_str()
            .expect("Asset address not found");
        let asset_contract = web3
            .eth()
            .contract(
                include_bytes!("YOUR_ASSET_CONTRACT_ABI_HERE"), // Replace with your asset contract ABI
                asset_address,
            );
        let balance: U256 = asset_contract
            .query("balanceOf", (), None, web3::transports::None)
            .await
            .expect("Failed to query balance");
        let diff = balance - base_asset.start_balance;
        let basis_points = (diff * 10000u64) / base_asset.start_balance;
        println!("#  {}: {}bps", base_asset.sym, basis_points);
    }
}

async fn look_for_dual_trade(
    config: Value,
    arb: web3::transports::ipc::Ipc,
    balances: Vec<BalanceInfo>,
    sender: mpsc::Sender<u64>,
    web3: Web3<Http>,
) {
    let good_count = 0;
    loop {
        let target_route = if config["routes"].as_array().unwrap().len() > 0 {
            use_good_routes(config, good_count)
        } else {
            search_for_routes(&config)
        };

        let trade_size = balances
            .iter()
            .find(|&balance_info| balance_info.balance > 0u64)
            .map(|balance_info| balance_info.balance)
            .unwrap_or(U256::zero());

        let amt_back: U256 = arb
            .call(
                TransactionRequest {
                    to: Some(target_route.router1.clone().into()),
                    data: target_route.router2.clone(),
                    gas: None,
                    gas_price: None,
                    value: None,
                    nonce: None,
                    condition: None,
                },
                web3::transports::None,
            )
            .await
            .expect("Failed to estimate dual dex trade");

        let multiplier = config["minBasisPointsPerTrade"].as_u64().unwrap() + 10000u64;
        let size_multiplied = trade_size * multiplier;
        let profit_target = size_multiplied / 10000u64;

        if !config["routes"].as_array().unwrap().len() > 0 {
            fs::append_file(
                format!("./data/{}_RouteLog.txt", network),
                format!(
                    r#"["{}","{}","{}","{}"],\n"#,
                    target_route.router1, target_route.router2, target_route.token1, target_route.token2,
                ),
            )
            .expect("Failed to append to log file");
        }

        if amt_back > profit_target {
            dual_trade(
                target_route.router1,
                target_route.router2,
                target_route.token1,
                target_route.token2,
                trade_size,
                &sender,
                &arb,
                &balances,
                web3,
            )
            .await;
        } else {
            sleep(Duration::from_secs(1)); // Wait for 1 second before retrying
        }
    }
}

fn use_good_routes(config: Value, good_count: u64) -> TargetRoute {
    let route = config["routes"][good_count as usize].as_array().unwrap();
    let target_route = TargetRoute {
        router1: route[0].as_str().unwrap().to_string(),
        router2:
