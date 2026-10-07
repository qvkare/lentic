// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

import {Script, console2} from "forge-std/Script.sol";
import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";

import {AttestationVerifier} from "../src/AttestationVerifier.sol";
import {IAutomataDcapAttestation} from "../src/interfaces/IAutomataDcapAttestation.sol";
import {LoanBook} from "../src/LoanBook.sol";

/// @notice Lentic's shared contracts on Base (mainnet 8453 / Sepolia 84532) with a real TDX
///         attestation: the policy signer registers with a quote from its dstack CVM.
/// @dev Env: DEPLOYER_KEY, DCAP_ATTESTATION (Automata's verifier on the target chain),
///      POLICY_MEASUREMENT (keccak256(MRTD‖RTMR0..3) read from the CVM with
///      `lentic-policy measurement --attestor dstack`), FEE_RECIPIENT, FEE_BPS (default 1000 =
///      10% of interest), REGISTRATION_TTL (seconds, default 30 days), USDC (defaults: Base
///      0x8335…2913, Base Sepolia 0x036C…CF7e). Lenders then create vaults with DeployVault.s.sol.
contract Deploy is Script {
    function run() external {
        uint256 key = vm.envUint("DEPLOYER_KEY");
        IAutomataDcapAttestation dcap = IAutomataDcapAttestation(vm.envAddress("DCAP_ATTESTATION"));
        bytes32 measurement = vm.envBytes32("POLICY_MEASUREMENT");
        address feeRecipient = vm.envAddress("FEE_RECIPIENT");
        uint16 feeBps = uint16(vm.envOr("FEE_BPS", uint256(1_000)));
        uint64 ttl = uint64(vm.envOr("REGISTRATION_TTL", uint256(30 days)));
        address usdc = vm.envOr("USDC", defaultUsdc());
        uint256 startBlock = block.number;

        vm.startBroadcast(key);
        AttestationVerifier verifier = new AttestationVerifier(dcap, measurement, ttl);
        LoanBook book = new LoanBook(verifier, IERC20(usdc), feeRecipient, feeBps);
        vm.stopBroadcast();

        string memory o = "deployments";
        vm.serializeUint(o, "chainId", block.chainid);
        vm.serializeUint(o, "startBlock", startBlock);
        vm.serializeAddress(o, "dcap", address(dcap));
        vm.serializeAddress(o, "usdc", usdc);
        vm.serializeBytes32(o, "policyMeasurement", measurement);
        vm.serializeAddress(o, "attestationVerifier", address(verifier));
        string memory json = vm.serializeAddress(o, "loanBook", address(book));
        vm.writeJson(json, string.concat("./deployments/", vm.toString(block.chainid), ".json"));
        console2.log("AttestationVerifier", address(verifier));
        console2.log("LoanBook           ", address(book));
    }

    function defaultUsdc() internal view returns (address) {
        if (block.chainid == 8453) return 0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913;
        if (block.chainid == 84532) return 0x036CbD53842c5426634e7929541eC2318f3dCF7e;
        revert("set USDC for this chain");
    }
}
