// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

/// @notice Reads measurement registers and report data from an Intel TDX DCAP quote (v4).
/// @dev Layout: 48-byte quote header, then the 584-byte TD10 report body:
///      TEE_TCB_SVN 16 | MRSEAM 48 | MRSIGNERSEAM 48 | SEAMATTRIBUTES 8 | TDATTRIBUTES 8 | XFAM 8 |
///      MRTD 48 | MRCONFIGID 48 | MROWNER 48 | MROWNERCONFIG 48 | RTMR0..3 4×48 | REPORTDATA 64.
///      dstack extends RTMR3 with the app's compose hash, so (MRTD, RTMR0..3) pins the exact
///      policy-signer image and configuration.
library TdxQuote {
    uint256 internal constant HEADER_LEN = 48;
    uint256 internal constant BODY_LEN = 584;
    uint256 internal constant MRTD_OFFSET = HEADER_LEN + 136;
    uint256 internal constant RTMR0_OFFSET = HEADER_LEN + 328;
    uint256 internal constant REPORT_DATA_OFFSET = HEADER_LEN + 520;

    error QuoteTooShort(uint256 length);
    error UnsupportedQuoteVersion(uint16 version);

    struct Report {
        bytes mrtd;
        bytes rtmr0;
        bytes rtmr1;
        bytes rtmr2;
        bytes rtmr3;
        bytes32 reportDataHigh; // REPORTDATA[0:32]
        bytes32 reportDataLow; // REPORTDATA[32:64]
    }

    function parse(bytes calldata quote) internal pure returns (Report memory r) {
        if (quote.length < HEADER_LEN + BODY_LEN) revert QuoteTooShort(quote.length);
        uint16 version = uint16(uint8(quote[0])) | (uint16(uint8(quote[1])) << 8);
        if (version != 4) revert UnsupportedQuoteVersion(version);
        r.mrtd = quote[MRTD_OFFSET:MRTD_OFFSET + 48];
        r.rtmr0 = quote[RTMR0_OFFSET:RTMR0_OFFSET + 48];
        r.rtmr1 = quote[RTMR0_OFFSET + 48:RTMR0_OFFSET + 96];
        r.rtmr2 = quote[RTMR0_OFFSET + 96:RTMR0_OFFSET + 144];
        r.rtmr3 = quote[RTMR0_OFFSET + 144:RTMR0_OFFSET + 192];
        r.reportDataHigh = bytes32(quote[REPORT_DATA_OFFSET:REPORT_DATA_OFFSET + 32]);
        r.reportDataLow = bytes32(quote[REPORT_DATA_OFFSET + 32:REPORT_DATA_OFFSET + 64]);
    }

    /// @notice keccak256(MRTD ‖ RTMR0 ‖ RTMR1 ‖ RTMR2 ‖ RTMR3): the policy signer's code identity.
    function measurement(Report memory r) internal pure returns (bytes32) {
        return keccak256(abi.encodePacked(r.mrtd, r.rtmr0, r.rtmr1, r.rtmr2, r.rtmr3));
    }
}
