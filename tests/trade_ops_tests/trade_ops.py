import asyncio
import random
import json
import os
import time

from web3 import Web3

# Load configurations based on the network
network = "aurora"  # Replace with the actual network name
config_file_path = f"../config/{network}.json"
with open(config_file_path, "r") as config_file:
    config = json.load(config_file)

print(f"Loaded {len(config['routes'])} routes")

async def setup():
    """
    Initialize the setup, including connecting to Ethereum, loading contract instances,
    and setting up balances.
    """
    global owner, arb, balances
    owner = Web3.toChecksumAddress("YOUR_OWNER_ADDRESS_HERE")  # Replace with your owner address

    # Connect to Ethereum using Web3
    web3 = Web3(Web3.HTTPProvider("YOUR_ETHEREUM_NODE_URL_HERE"))  # Replace with your Ethereum node URL

    # Load the Arb contract instance
    arb_contract_address = Web3.toChecksumAddress(config["arbContract"])
    arb_contract_abi = json.loads("YOUR_ARB_CONTRACT_ABI_HERE")  # Replace with your Arb contract ABI
    arb = web3.eth.contract(address=arb_contract_address, abi=arb_contract_abi)

    # Initialize balances dictionary
    balances = {}
    for base_asset in config["baseAssets"]:
        asset_address = Web3.toChecksumAddress(base_asset["address"])
        asset_contract_abi = json.loads("YOUR_ASSET_CONTRACT_ABI_HERE")  # Replace with your asset contract ABI
        asset_contract = web3.eth.contract(address=asset_address, abi=asset_contract_abi)
        balance = asset_contract.functions.balanceOf(arb_contract_address).call()
        print(base_asset["sym"], balance)
        balances[asset_address] = {"sym": base_asset["sym"], "balance": balance, "startBalance": balance}

    # Schedule periodic logging of results
    time.sleep(120)  # Initial delay
    while True:
        log_results()
        time.sleep(600)  # Log results every 10 minutes

def log_results():
    """
    Log the difference in balances for each asset and calculate basis points.
    """
    print("############# LOGS #############")
    for base_asset in config["baseAssets"]:
        asset_address = Web3.toChecksumAddress(base_asset["address"])
        asset_contract_abi = json.loads("YOUR_ASSET_CONTRACT_ABI_HERE")  # Replace with your asset contract ABI
        asset_contract = web3.eth.contract(address=asset_address, abi=asset_contract_abi)
        balances[asset_address]["balance"] = asset_contract.functions.balanceOf(arb_contract_address).call()
        diff = balances[asset_address]["balance"] - balances[asset_address]["startBalance"]
        basis_points = diff * 10000 / balances[asset_address]["startBalance"]
        print(f"#  {base_asset['sym']}: {basis_points}bps")

async def look_for_dual_trade():
    """
    Search for dual trade opportunities and execute them if profitable.
    """
    while True:
        target_route = use_good_routes() if config["routes"] else search_for_routes()
        try:
            trade_size = next((balance["balance"] for balance in balances.values() if balance["balance"] > 0), 0)
            amt_back = await arb.functions.estimateDualDexTrade(
                target_route["router1"], target_route["router2"],
                target_route["token1"], target_route["token2"], trade_size
            ).call()
            multiplier = config["minBasisPointsPerTrade"] + 10000
            size_multiplied = trade_size * multiplier
            profit_target = size_multiplied / 10000
            if not config["routes"]:
                with open(f"./data/{network}RouteLog.txt", "a") as log_file:
                    log_file.write(
                        f'["{target_route["router1"]}","{target_route["router2"]}",'
                        f'"{target_route["token1"]}","{target_route["token2}"],\n'
                    )
            if amt_back > profit_target:
                await dual_trade(target_route["router1"], target_route["router2"],
                                 target_route["token1"], target_route["token2"], trade_size)
            else:
                await asyncio.sleep(1)  # Wait for 1 second before retrying
        except Exception as e:
            print(e)
            await asyncio.sleep(1)

async def dual_trade(router1, router2, base_token, token2, amount):
    """
    Execute a dual trade if conditions are met.
    """
    global in_trade
    if in_trade:
        await asyncio.sleep(1)
        return False
    try:
        in_trade = True
        print('> Making dualTrade...')
        tx = await arb.connect(owner).dualDexTrade(router1, router2, base_token, token2, amount)
        await tx.wait()
        in_trade = False
        await look_for_dual_trade()
    except Exception as e:
        print(e)
        in_trade = False
        await asyncio.sleep(1)

def search_for_routes():
    """
    Generate a random route for trading.
    """
    target_route = {}
    target_route["router1"] = random.choice(config["routers"])["address"]
    target_route["router2"] = random.choice(config["routers"])["address"]
    target_route["token1"] = random.choice(config["baseAssets"])["address"]
    target_route["token2"] = random.choice(config["tokens"])["address"]
    return target_route

def use_good_routes():
    """
    Use a route from the configured list of routes.
    """
    global good_count
    route = config["routes"][good_count]
    good_count = (good_count + 1) % len(config["routes"])
    target_route = {
        "router1": route[0],
        "router2": route[1],
        "token1": route[2],
        "token2": route[3]
    }
    return target_route

if __name__ == "__main__":
    # Initialize global variables
    owner, arb, balances, in_trade, good_count = None, None, None, False, 0

    # Start the main asyncio event loop
    loop = asyncio.get_event_loop()
    loop.run_until_complete(setup())
    loop.run_until_complete(look_for_dual_trade())
