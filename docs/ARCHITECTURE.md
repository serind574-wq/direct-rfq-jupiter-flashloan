# Architecture

## Roles

```text
Protocol authority
  Owns global pause and maker-pair admission.

Maker AB
  Owns an A -> B pair, its vault inventory, and posts direct on-chain quotes.

Maker BA
  Owns a B -> A pair, its vault inventory, and posts direct on-chain quotes.

Taker / executor
  Creates RFQs, composes Jupiter Lend with the two selected fills, and pays transaction fees.

Jupiter Lend
  Supplies temporary asset A and requires A + current flash fee before transaction completion.
```

The same organization can operate both maker pairs for a laboratory experiment, but this
makes the RFQ transfers internal inventory movements. Any economic profit still comes from
a real price discrepancy or intentionally configured test quotes.

## On-chain state

```text
ProtocolConfig PDA
  authority, max request TTL, global pause.

MakerPairConfig PDA: maker + input mint + output mint
  pair limits, PDA vault addresses, maker owner, reserved output balance.

Vault authority PDA: maker pair
  authority over pair input and output token vault ATAs.

RfqRequest PDA: taker + request ID
  exact input, min output, expiry, token accounts, selected fixed maker pair.

Quote PDA: RFQ
  quoted output, quote expiry, quote state.
```

## Inventory reservations

At `post_quote`, the program enforces:

```text
free output inventory = vault balance - reserved output amount
quoted output <= free output inventory
```

It then increases `reserved_output_amount`. This prevents two open quotes from both
claiming the same vault balance. A fill, explicit maker cancellation, or permissionless
post-expiry release decrements the reservation.

## Atomic final transaction

```text
borrow A
  ↓
fill quote AB: A from executor → maker AB input vault; B from maker AB output vault → executor
  ↓
fill quote BA: B from executor → maker BA input vault; A from maker BA output vault → executor
  ↓
assert A balance floor
  ↓
payback A + Jupiter-reported flash fee
```

The RFQ program does not invoke Jupiter Lend. The executor client serializes all top-level
instructions in the correct order. This avoids coupling the RFQ program to a mutable
external lending SDK.

## Security boundaries

| Boundary | Enforcement |
|---|---|
| Taker cannot select a different maker at fill | RFQ stores fixed `maker_pair`; fill checks it. |
| Taker cannot alter price | Quote stores output amount and is made only by registered maker signer. |
| Maker cannot over-quote vault | On-chain reservation accounting. |
| Quote cannot fill twice | RFQ and Quote status transition `Open → Filled`. |
| Quote cannot fill after expiry | Current slot checked at fill. |
| Maker cannot withdraw reserved output | Withdrawal checks `vault.amount - reserved`. |
| Flash loan cannot remain unpaid | Jupiter Lend payback is in same atomic transaction. |
| Bad round trip cannot settle | `assert_min_token_balance` runs before payback. |

## Why it is direct RFQ and not an aggregator

Each RFQ stores exactly one maker pair. The executor explicitly chooses `MakerPair AB` and
`MakerPair BA`; the program does not solicit, rank, split, or route among competing makers.
