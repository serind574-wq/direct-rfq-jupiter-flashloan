# Direct on-chain RFQ × Jupiter Lend flash-loan lab

> **Educational development scaffold — not audited and not safe for mainnet capital.**
>
> It demonstrates the architecture requested here: two direct, fixed-maker RFQs are
> **posted on-chain before** the final transaction. The final transaction composes:
>
> `Jupiter Lend flash borrow A → RFQ fill A→B → RFQ fill B→A → profit assertion → Jupiter payback`.

This project deliberately does **not** use an aggregator or JupiterZ RFQ. Jupiter Lend
is used only to construct the top-level flash-loan borrow/payback instructions; the
Anchor program provides the direct RFQ settlement legs.

## What is on-chain vs off-chain

| Component | Where it runs |
|---|---|
| RFQ request | On-chain `create_rfq` transaction |
| MM executable quote | On-chain `post_quote` transaction, signed by fixed maker wallet |
| Maker inventory | On-chain PDA-owned SPL token vault |
| RFQ settlement | On-chain `fill_quote` instruction |
| Flash borrow and payback | Jupiter Lend top-level instructions |
| Opportunity detection and quote pricing | Off-chain bot / MM operator |
| Final transaction assembly and sending | Off-chain executor client |

The maker's price calculation may be private/off-chain, but its selected quote is posted
on-chain before the flash-loan transaction. This avoids an HTTP RFQ call during the flash
loan and provides a useful study model for an on-chain RFQ lifecycle.

## Transaction lifecycle

```text
Preparation — separate on-chain transactions

  1. Taker: create RFQ #1, exact input A, wants B
  2. Maker AB: post quote #1, reserves B in its vault
  3. Taker: create RFQ #2, exact input equals quote #1 output B, wants A
  4. Maker BA: post quote #2, reserves A in its vault

Atomic execution — one transaction

  ComputeBudget
  → Jupiter Lend flash borrow A
  → RFQ program fill quote #1: A → B
  → RFQ program fill quote #2: B → A
  → RFQ program assert final A >= initial A + loan + fee + profit
  → Jupiter Lend flash payback A
```

If any final instruction fails, Solana reverts the entire final transaction.

## Program model

`programs/direct_rfq_arb` implements:

- `initialize_protocol`: creates global configuration.
- `register_maker_pair`: creates fixed maker/pair state and PDA-owned input/output vault ATAs.
- `deposit_output_inventory`: funds the maker asset that will be delivered to takers.
- `create_rfq`: a taker posts a request with exact input, minimum output, and slot expiry.
- `post_quote`: the registered maker posts a single executable quote and reserves output inventory.
- `fill_quote`: transfers taker input to maker proceeds vault and maker output to taker.
- `assert_min_token_balance`: on-chain profit floor used before Jupiter payback.
- quote cancellation/expiry and maker withdrawals that respect open quote reservations.

The MVP uses **legacy SPL Token Program only**, fixed pair directions, one quote per RFQ,
and exact-input/full-fill semantics. It intentionally excludes Token-2022 extensions,
partial fills, routing, and arbitrary CPI.

## Prerequisites

Install these locally:

```bash
# Rust + Solana CLI + Anchor. Version pinning is deliberate for this lab.
rustup toolchain install stable
sh -c "$(curl -sSfL https://release.anza.xyz/stable/install)"
# Install Anchor via AVM, or use your team-standard Anchor 0.31.1 install.
# https://www.anchor-lang.com/docs/installation

node --version # use Node 20+
npm --version
```

Then:

```bash
cd direct-rfq-jupiter-flashloan
npm install
cp .env.example .env
anchor build
anchor keys list
```

After `anchor keys list`, replace the placeholder program ID in both locations:

```text
programs/direct_rfq_arb/src/lib.rs     declare_id!(...)
Anchor.toml                            [programs.localnet].direct_rfq_arb
```

Run `anchor build` again after changing the ID.

## Local study workflow

The RFQ program can be studied locally with test mints. Jupiter Lend flash loans are a
mainnet integration, so local/devnet validates RFQ lifecycle and token accounting, not
live Jupiter liquidity.

```bash
solana config set --url localhost
solana-test-validator --reset
anchor build
anchor deploy
```

Create two local SPL mints and associated token accounts using `spl-token`. Treat them as:

```text
A = test USDC
B = test wSOL
```

Then use the scripts below to initialize protocol, register pairs, fund maker vaults,
create requests, and post quotes. See [`docs/RUNBOOK.md`](docs/RUNBOOK.md) for exact commands.

## Mainnet Jupiter Lend adapter

The client in `scripts/execute-jupiter-lend-arb.ts` uses the current Jupiter Lend SDK:

```ts
import { getFlashloanIx } from "@jup-ag/lend/flashloan";
```

It obtains Jupiter's `borrowIx`, `paybackIx`, and runtime-reported `feeAmount`, then
places the program's two `fill_quote` instructions between them. It simulates before it
will submit. Submission is blocked unless:

```text
ALLOW_MAINNET_SUBMISSION=true
```

The script does **not** assume the loan is free even if a documentation page or a specific
market currently advertises zero fees; it takes `feeAmount` from the installed SDK and
requires the final balance to cover principal + SDK fee + configured profit floor.

Review Jupiter's current flash-loan documentation before every deployment:

- <https://developers.jup.ag/docs/lend/flashloan>
- <https://developers.jup.ag/docs/lend/flashloan/execute>

## Critical limitations and safety rules

1. **Do not deploy this code with real maker funds.** It has not been audited.
2. This is a **two-RFQ on-chain quote lab**, not an integration with a public MM. A real
   MM must agree to register/fund a vault and post quotes into your program.
3. Jupiter Lend availability, supported asset mints, fees, market names, instruction
   schemas, and account constraints can change. The SDK and live simulation are source of truth.
4. RFQ quote expiry is slot-based. Final fills must land before both RFQs expire.
5. The executor's RFQ input account must match Jupiter Lend's `signerBorrowTokenAccount`.
6. The program has no protection against an unaudited upgrade authority. Use a multisig
   and an external audit before any real integration.
7. This design is slower and more public than a maker-signed off-chain quote because RFQs
   must be posted before the flash transaction. That is intentional for this study goal.

## Next steps

1. Follow [`docs/RUNBOOK.md`](docs/RUNBOOK.md) on localnet.
2. Add Anchor integration tests for every failure condition in
   [`docs/TEST_PLAN.md`](docs/TEST_PLAN.md).
3. Use a test-only maker operator to post quotes with deliberate profitable and
   unprofitable outcomes.
4. Read [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) before adapting it.
5. Only then configure a mainnet RPC, real Jupiter Lend market, and a no-submit simulation.
