# Runbook: local study, then Jupiter no-submit simulation

## A. Build and deploy locally

```bash
cd direct-rfq-jupiter-flashloan
npm install
cp .env.example .env

solana config set --url localhost
solana-keygen new --outfile ~/.config/solana/id.json
solana airdrop 10
solana-test-validator --reset

# In another terminal
anchor build
anchor keys list
# Copy the generated program ID into lib.rs and Anchor.toml, then rebuild.
anchor build
anchor deploy
```

Set `.env` for localnet:

```dotenv
RPC_URL=http://127.0.0.1:8899
KEYPAIR_PATH=/absolute/path/to/.config/solana/id.json
RFQ_PROGRAM_ID=<deployed program id>
MAX_REQUEST_TTL_SLOTS=100
```

## B. Create study mints/accounts

Use legacy SPL mints only:

```bash
A_MINT=$(spl-token create-token | awk '/Creating token/ {print $3}')
B_MINT=$(spl-token create-token | awk '/Creating token/ {print $3}')
A_ATA=$(spl-token create-account "$A_MINT" | awk '/Creating account/ {print $3}')
B_ATA=$(spl-token create-account "$B_MINT" | awk '/Creating account/ {print $3}')

# Lab balances; use arbitrary sizes.
spl-token mint "$A_MINT" 1000000 "$A_ATA"
spl-token mint "$B_MINT" 1000000 "$B_ATA"
```

## C. Initialize protocol

```bash
MAX_REQUEST_TTL_SLOTS=100 npm run init-protocol
```

## D. Register two direct maker pairs

For a single-key laboratory maker, authority and maker use the same keypair.

```bash
# Pair AB: taker pays A and maker delivers B.
INPUT_MINT="$A_MINT" \
OUTPUT_MINT="$B_MINT" \
MAX_INPUT_AMOUNT=1000000000000 \
npm run register-maker-pair
```

Record `makerPair` from output as `PAIR_AB`.

```bash
# Pair BA: taker pays B and maker delivers A.
INPUT_MINT="$B_MINT" \
OUTPUT_MINT="$A_MINT" \
MAX_INPUT_AMOUNT=1000000000000 \
npm run register-maker-pair
```

Record this as `PAIR_BA`.

## E. Fund maker output vaults

First obtain the maker's ordinary source ATAs if they differ from your executor ATAs.

```bash
# Pair AB needs B inventory.
MAKER_PAIR="$PAIR_AB" \
MAKER_OUTPUT_SOURCE="$B_ATA" \
DEPOSIT_AMOUNT=100000 \
npm run deposit-output-inventory

# Pair BA needs A inventory.
MAKER_PAIR="$PAIR_BA" \
MAKER_OUTPUT_SOURCE="$A_ATA" \
DEPOSIT_AMOUNT=100000 \
npm run deposit-output-inventory
```

In a realistic setup, maker vault inventory and executor balances should use separate
keypairs and accounts. One-key mode is only for studying instruction/state flow.

## F. Post the two on-chain RFQs

### RFQ 1: A → B

```bash
MAKER_PAIR="$PAIR_AB" \
TAKER_INPUT_TOKEN_ACCOUNT="$A_ATA" \
TAKER_OUTPUT_TOKEN_ACCOUNT="$B_ATA" \
REQUEST_ID=1 \
INPUT_AMOUNT=1000 \
MIN_OUTPUT_AMOUNT=1000 \
RFQ_TTL_SLOTS=30 \
npm run create-rfq
```

Maker posts an executable quote. For this local experiment, use a deliberately favorable
quote only if you want the round trip to satisfy the later profit guard.

```bash
# If maker uses a different keypair: KEYPAIR_PATH=<maker key> and TAKER_PUBKEY=<executor pubkey>
MAKER_PAIR="$PAIR_AB" \
TAKER_PUBKEY=$(solana address) \
REQUEST_ID=1 \
QUOTE_OUTPUT_AMOUNT=1100 \
QUOTE_TTL_SLOTS=12 \
npm run post-quote
```

### RFQ 2: B → A

Its `INPUT_AMOUNT` must exactly equal RFQ 1's posted `QUOTE_OUTPUT_AMOUNT`.

```bash
MAKER_PAIR="$PAIR_BA" \
TAKER_INPUT_TOKEN_ACCOUNT="$B_ATA" \
TAKER_OUTPUT_TOKEN_ACCOUNT="$A_ATA" \
REQUEST_ID=2 \
INPUT_AMOUNT=1100 \
MIN_OUTPUT_AMOUNT=1001 \
RFQ_TTL_SLOTS=30 \
npm run create-rfq

MAKER_PAIR="$PAIR_BA" \
TAKER_PUBKEY=$(solana address) \
REQUEST_ID=2 \
QUOTE_OUTPUT_AMOUNT=1010 \
QUOTE_TTL_SLOTS=12 \
npm run post-quote
```

The final mainnet adapter checks that RFQ 1 output equals RFQ 2 input before constructing
the flash transaction.

## G. Jupiter Lend no-submit simulation

This requires:

- deployment of the RFQ program on mainnet;
- real, legacy-SPL compatible asset accounts;
- currently supported Jupiter Lend flash asset;
- two live, unexpired on-chain RFQ quotes;
- a funded fee-payer wallet.

Set mainnet values in `.env`:

```dotenv
RPC_URL=https://your-low-latency-mainnet-rpc
KEYPAIR_PATH=/absolute/path/to/executor.json
RFQ_PROGRAM_ID=<mainnet deployment>
FLASH_ASSET_MINT=<supported Jupiter flash asset>
FLASH_AMOUNT=<base units>
MIN_PROFIT_A=<base units>
MAKER_PAIR_AB=<A-to-B pair PDA>
MAKER_PAIR_BA=<B-to-A pair PDA>
RFQ_ONE_ID=1
RFQ_TWO_ID=2
TAKER_A_TOKEN_ACCOUNT=<Jupiter signerBorrowTokenAccount>
TAKER_B_TOKEN_ACCOUNT=<executor B ATA>
JUPITER_LEND_MARKET=main
ALLOW_MAINNET_SUBMISSION=false
```

Run a simulation only:

```bash
npm run execute-jupiter-arb
```

The script will stop on any account mismatch, non-profitable quote pair, or failed
simulation. It does not submit until you set `ALLOW_MAINNET_SUBMISSION=true`.

## H. Account ordering check

Jupiter's SDK builds the flash borrow/payback instructions. The program does not need to
know Jupiter's internal accounts. The required order is:

```text
compute budget
→ Jupiter borrow
→ fill RFQ #1
→ fill RFQ #2
→ profit assertion
→ Jupiter payback
```

If Jupiter changes its flashloan API, re-read its docs and compare the SDK's generated
instructions before running any transaction.
