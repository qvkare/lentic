// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

import {Script, console2} from "forge-std/Script.sol";

import {AttestationVerifier} from "../src/AttestationVerifier.sol";
import {LenderVault} from "../src/LenderVault.sol";
import {LoanBook} from "../src/LoanBook.sol";
import {DevQuote} from "../src/mocks/DevQuote.sol";
import {MockDcapAttestation} from "../src/mocks/MockDcapAttestation.sol";
import {MockUSDC} from "../src/mocks/MockUSDC.sol";

/// @notice Full local stack on anvil (chain 31337) with the default anvil accounts:
///         #0 deployer + Lentic fee recipient, #1 lender, #2 borrower, #3 dev policy signer.
///         Writes deployments/31337.json for the services and the web app.
contract DeployLocal is Script {
    uint256 constant DEPLOYER_KEY = 0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80;
    uint256 constant LENDER_KEY = 0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d;
    uint256 constant BORROWER_KEY = 0x5de4111afa1a4b94908f83103eb1f1706367c2e68ca870fc3fb9a804cdab365a;
    uint256 constant POLICY_KEY = 0x7c852118294e51e653712a81e05800f419141751be58f605c371e15141b007a6;

    /// Must match lentic_policy's dev config hash (keccak256("lentic-policy-dev")).
    bytes32 constant DEV_CONFIG_HASH = keccak256("lentic-policy-dev");

    function run() external {
        address deployer = vm.addr(DEPLOYER_KEY);
        address lender = vm.addr(LENDER_KEY);
        address borrower = vm.addr(BORROWER_KEY);
        address policy = vm.addr(POLICY_KEY);
        uint256 startBlock = block.number;

        vm.startBroadcast(DEPLOYER_KEY);
        MockUSDC usdc = new MockUSDC();
        MockDcapAttestation dcap = new MockDcapAttestation();
        AttestationVerifier verifier = new AttestationVerifier(dcap, DevQuote.measurement(), 30 days);
        LoanBook book = new LoanBook(verifier);
        verifier.register(DevQuote.build(policy, DEV_CONFIG_HASH));
        usdc.mint(lender, 1_000_000e6);
        usdc.mint(borrower, 50_000e6);
        vm.stopBroadcast();

        vm.startBroadcast(LENDER_KEY);
        LenderVault vault =
            new LenderVault(usdc, "Lentic Lender Vault A", "lvUSDC-A", lender, book, deployer, 1_000);
        usdc.approve(address(vault), type(uint256).max);
        vault.deposit(500_000e6, lender);
        vm.stopBroadcast();

        string memory o = "deployments";
        vm.serializeUint(o, "chainId", block.chainid);
        vm.serializeUint(o, "startBlock", startBlock);
        vm.serializeAddress(o, "usdc", address(usdc));
        vm.serializeAddress(o, "dcap", address(dcap));
        vm.serializeAddress(o, "attestationVerifier", address(verifier));
        vm.serializeAddress(o, "loanBook", address(book));
        vm.serializeAddress(o, "lenderVault", address(vault));
        vm.serializeAddress(o, "feeRecipient", deployer);
        vm.serializeAddress(o, "lender", lender);
        vm.serializeAddress(o, "borrower", borrower);
        vm.serializeBytes32(o, "devConfigHash", DEV_CONFIG_HASH);
        string memory json = vm.serializeAddress(o, "policySigner", policy);
        vm.writeJson(json, "./deployments/31337.json");

        console2.log("LoanBook     ", address(book));
        console2.log("LenderVault  ", address(vault));
        console2.log("Verifier     ", address(verifier));
        console2.log("USDC (mock)  ", address(usdc));
    }
}
