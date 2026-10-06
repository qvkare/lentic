// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

import {Script, console2} from "forge-std/Script.sol";

import {AttestationVerifier} from "../src/AttestationVerifier.sol";
import {IAutomataDcapAttestation} from "../src/interfaces/IAutomataDcapAttestation.sol";
import {LoanBook} from "../src/LoanBook.sol";

/// @notice Lentic's shared contracts on Base (mainnet 8453 / Sepolia 84532).
/// @dev Env: DEPLOYER_KEY, DCAP_ATTESTATION (Automata AutomataDcapAttestationFee on the target
///      chain), POLICY_MEASUREMENT (keccak256(MRTD‖RTMR0..3) of the released policy image, from
///      `lentic-policy measurement`), REGISTRATION_TTL (seconds, default 30 days).
///      Lender vaults are deployed by each lender with DeployVault.s.sol.
contract Deploy is Script {
    function run() external {
        uint256 key = vm.envUint("DEPLOYER_KEY");
        IAutomataDcapAttestation dcap = IAutomataDcapAttestation(vm.envAddress("DCAP_ATTESTATION"));
        bytes32 measurement = vm.envBytes32("POLICY_MEASUREMENT");
        uint64 ttl = uint64(vm.envOr("REGISTRATION_TTL", uint256(30 days)));
        uint256 startBlock = block.number;

        vm.startBroadcast(key);
        AttestationVerifier verifier = new AttestationVerifier(dcap, measurement, ttl);
        LoanBook book = new LoanBook(verifier);
        vm.stopBroadcast();

        string memory o = "deployments";
        vm.serializeUint(o, "chainId", block.chainid);
        vm.serializeUint(o, "startBlock", startBlock);
        vm.serializeAddress(o, "dcap", address(dcap));
        vm.serializeBytes32(o, "policyMeasurement", measurement);
        vm.serializeAddress(o, "attestationVerifier", address(verifier));
        string memory json = vm.serializeAddress(o, "loanBook", address(book));
        vm.writeJson(json, string.concat("./deployments/", vm.toString(block.chainid), ".json"));
        console2.log("AttestationVerifier", address(verifier));
        console2.log("LoanBook           ", address(book));
    }
}
