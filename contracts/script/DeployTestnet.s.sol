// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

import {Script, console2} from "forge-std/Script.sol";
import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";

import {AttestationVerifier} from "../src/AttestationVerifier.sol";
import {LenderVault} from "../src/LenderVault.sol";
import {LoanBook} from "../src/LoanBook.sol";
import {DevQuote} from "../src/mocks/DevQuote.sol";
import {MockDcapAttestation} from "../src/mocks/MockDcapAttestation.sol";
import {MockUSDC} from "../src/mocks/MockUSDC.sol";

/// @notice Lentic on a public testnet (Base Sepolia, 84532) with the development attestor.
///         LoanBook, LenderVault and AttestationVerifier are the production contracts; what
///         differs is the attestation: no TDX CVM signs a quote yet, so a mock DCAP verifier
///         accepts the policy signer's development quote. Anyone can register a development
///         quote with it, so on this deployment the attestation proves nothing; Deploy.s.sol is
///         the real thing. USDC is a mock token unless USDC names one.
/// @dev Env: DEPLOYER_KEY (pays gas, receives Lentic's fee), LENDER_KEY (creates and owns the
///      vault), POLICY_SIGNER (the policy signer's Base address), BORROWER (receives mock USDC
///      for repayments), VAULT_DEPOSIT (default 100 USDC), USDC (optional existing token).
///      Writes deployments/<chainId>.json.
contract DeployTestnet is Script {
    /// Must match lentic_policy's dev config hash (keccak256("lentic-policy-dev")).
    bytes32 constant DEV_CONFIG_HASH = keccak256("lentic-policy-dev");

    function run() external {
        uint256 deployerKey = vm.envUint("DEPLOYER_KEY");
        uint256 lenderKey = vm.envUint("LENDER_KEY");
        address deployer = vm.addr(deployerKey);
        address lender = vm.addr(lenderKey);
        address policy = vm.envAddress("POLICY_SIGNER");
        address borrower = vm.envAddress("BORROWER");
        address existingUsdc = vm.envOr("USDC", address(0));
        uint256 deposit = vm.envOr("VAULT_DEPOSIT", uint256(100e6));
        uint256 startBlock = block.number;

        vm.startBroadcast(deployerKey);
        address usdc = existingUsdc;
        if (usdc == address(0)) {
            MockUSDC mock = new MockUSDC();
            mock.mint(lender, deposit);
            mock.mint(borrower, 50e6);
            usdc = address(mock);
        }
        MockDcapAttestation dcap = new MockDcapAttestation();
        AttestationVerifier verifier = new AttestationVerifier(dcap, DevQuote.measurement(), 30 days);
        LoanBook book = new LoanBook(verifier, IERC20(usdc), deployer, 1_000);
        verifier.register(DevQuote.build(policy, DEV_CONFIG_HASH));
        vm.stopBroadcast();

        vm.startBroadcast(lenderKey);
        LenderVault vault = book.createVault("Lentic Lender Vault (testnet)", "lvUSDC-T");
        IERC20(usdc).approve(address(vault), deposit);
        vault.deposit(deposit, lender);
        vm.stopBroadcast();

        string memory o = "deployments";
        vm.serializeUint(o, "chainId", block.chainid);
        vm.serializeUint(o, "startBlock", startBlock);
        vm.serializeAddress(o, "usdc", usdc);
        vm.serializeBool(o, "usdcIsMock", existingUsdc == address(0));
        vm.serializeAddress(o, "dcap", address(dcap));
        vm.serializeString(o, "attestor", "dev");
        vm.serializeAddress(o, "attestationVerifier", address(verifier));
        vm.serializeAddress(o, "loanBook", address(book));
        vm.serializeAddress(o, "lenderVault", address(vault));
        vm.serializeAddress(o, "feeRecipient", deployer);
        vm.serializeAddress(o, "lender", lender);
        vm.serializeAddress(o, "borrower", borrower);
        vm.serializeBytes32(o, "devConfigHash", DEV_CONFIG_HASH);
        string memory json = vm.serializeAddress(o, "policySigner", policy);
        vm.writeJson(json, string.concat("./deployments/", vm.toString(block.chainid), ".json"));

        console2.log("LoanBook      ", address(book));
        console2.log("LenderVault   ", address(vault));
        console2.log("Verifier      ", address(verifier));
        console2.log("USDC          ", usdc);
    }
}
