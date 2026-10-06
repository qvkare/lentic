// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

import {Script, console2} from "forge-std/Script.sol";
import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";

import {LenderVault} from "../src/LenderVault.sol";
import {LoanBook} from "../src/LoanBook.sol";

/// @notice A lender deploys its own vault. The broadcasting key becomes the owner.
/// @dev Env: LENDER_KEY, LOAN_BOOK, FEE_RECIPIENT, FEE_BPS (default 1000 = 10% of interest),
///      VAULT_NAME, VAULT_SYMBOL, USDC (defaults: Base 0x8335…2913, Base Sepolia 0x036C…CF7e).
contract DeployVault is Script {
    function run() external {
        uint256 key = vm.envUint("LENDER_KEY");
        address usdc = vm.envOr("USDC", defaultUsdc());
        LoanBook book = LoanBook(vm.envAddress("LOAN_BOOK"));
        address feeRecipient = vm.envAddress("FEE_RECIPIENT");
        uint16 feeBps = uint16(vm.envOr("FEE_BPS", uint256(1_000)));
        string memory name = vm.envOr("VAULT_NAME", string("Lentic Lender Vault"));
        string memory symbol = vm.envOr("VAULT_SYMBOL", string("lvUSDC"));

        vm.startBroadcast(key);
        LenderVault vault =
            new LenderVault(IERC20(usdc), name, symbol, vm.addr(key), book, feeRecipient, feeBps);
        vm.stopBroadcast();
        console2.log("LenderVault", address(vault));
    }

    function defaultUsdc() internal view returns (address) {
        if (block.chainid == 8453) return 0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913;
        if (block.chainid == 84532) return 0x036CbD53842c5426634e7929541eC2318f3dCF7e;
        revert("set USDC for this chain");
    }
}
