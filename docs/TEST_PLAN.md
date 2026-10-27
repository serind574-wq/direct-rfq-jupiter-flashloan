# Required test plan before any real use

## Program unit/integration tests

- Protocol initialization can happen once only.
- Only protocol authority can register a maker pair.
- Pair input/output mint equality is rejected.
- Maker output deposit increases vault balance.
- Taker cannot create RFQ above pair maximum input.
- RFQ with expired or excessive TTL is rejected.
- Only registered maker owner can post quote.
- Quote below RFQ minimum is rejected.
- Quote after RFQ expiry is rejected.
- Quote exceeding free inventory is rejected.
- A quote increases reserved output amount.
- Maker cannot withdraw reserved output inventory.
- A valid fill sends input to maker input vault and output to taker.
- Re-filling a quote fails.
- Filling after RFQ or quote expiry fails.
- Cancelling/exiring a quote releases reserved inventory exactly once.
- Wrong taker token accounts fail.
- Wrong vault accounts fail.
- `assert_min_token_balance` fails below threshold.
- If a subsequent instruction fails, both RFQ transfers revert atomically.

## Jupiter adapter tests

- Asset A equals RFQ 1 input and RFQ 2 output.
- RFQ 1 output equals RFQ 2 input.
- SDK `signerBorrowTokenAccount` equals RFQ 1 input account.
- Quote statuses are `Open` and unexpired when building.
- Quote 2 output is at least principal + SDK-reported fee + profit floor.
- Simulation succeeds before submission.
- Any simulation error blocks submission.
- `ALLOW_MAINNET_SUBMISSION` must explicitly equal `true`.

## Security review

- Audit PDA signer seed derivation.
- Audit every vault/mint/owner constraint.
- Use checked arithmetic only.
- Validate token-program assumptions; this MVP is legacy SPL only.
- Put deployment upgrade authority under multisig.
- Add a maker-pair pause path and incident runbook.
- Do not accept arbitrary program IDs or arbitrary CPI payloads from callers.
