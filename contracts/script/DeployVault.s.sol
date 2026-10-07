// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

import {Script, console2} from "forge-std/Script.sol";

import {LenderVault} from "../src/LenderVault.sol";
import {LoanBook} from "../src/LoanBook.sol";

/// @notice A lender creates its own vault through LoanBook (only such vaults can open loans).
///         The broadcasting key becomes the owner; the USDC and Lentic's fee come from the book.
/// @dev Env: LENDER_KEY, LOAN_BOOK, VAULT_NAME, VAULT_SYMBOL.
contract DeployVault is Script {
    function run() external {
        uint256 key = vm.envUint("LENDER_KEY");
        LoanBook book = LoanBook(vm.envAddress("LOAN_BOOK"));
        string memory name = vm.envOr("VAULT_NAME", string("Lentic Lender Vault"));
        string memory symbol = vm.envOr("VAULT_SYMBOL", string("lvUSDC"));

        vm.startBroadcast(key);
        LenderVault vault = book.createVault(name, symbol);
        vm.stopBroadcast();
        console2.log("LenderVault", address(vault));
    }
}
