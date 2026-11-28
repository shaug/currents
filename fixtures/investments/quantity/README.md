# Synthetic trade inputs

The CUR-021 trade and holdings tests in
`crates/currents-domain/tests/cur_021.rs` import these files. Every account,
security and amount in them is invented.

Each file uses the `SyntheticCsv` import profile, which takes one unit per
file:

- `cash-b.csv` holds USD movement rows on cash account B.
- `quantity-q.csv` holds SECA movement rows on quantity account Q. SECA is a
  synthetic security unit with native code `SCA` and exponent 4.

Rows with the same `original_id` in the two files are the two sides of one
trade:

| Row            | Cash on B | Quantity on Q | Date       |
| -------------- | --------- | ------------- | ---------- |
| `buy`          | −1,000.00 | +10           | 2026-11-03 |
| `sale`         | +517.00   | −4            | 2026-11-10 |
| `buy-with-fee` | −1,005.00 | +10           | 2026-11-03 |

The sale's cash row is its proceeds of 520.00 minus a 3.00 selling fee. The
`buy-with-fee` cash row is a cost of 1,000.00 plus a 5.00 fee.

The `actions-*.csv` files hold the inputs for the CUR-021 reinvested
dividend, split and corporate action tests. `actions-cash-b.csv` holds USD
rows on B, `actions-quantity-q.csv` holds SECA rows on Q and
`actions-quantity-qw.csv` holds rows on QW in SECW, a synthetic security unit
with native code `SCW` and exponent 0.

| Row            | File | Amount       | Date       | Use                                    |
| -------------- | ---- | ------------ | ---------- | -------------------------------------- |
| `buy-p`        | B, Q | −600.00, +6  | 2026-11-02 | the buy that gives P 6 SECA            |
| `dividend`     | B    | +50.00       | 2026-11-15 | the SECA dividend                      |
| `reinvest`     | B, Q | −50.00, +0.4 | 2026-11-15 | the buy the dividend pays for          |
| `late`         | Q    | +1           | 2026-11-10 | a buy reported after the split         |
| `merger`       | Q    | −6           | 2026-11-25 | a merger the engine cannot apply       |
| `cash-in-lieu` | B    | +30.00       | 2026-11-20 | cash for the 0.5 SECW the split leaves |
| `buy-w`        | QW   | +5           | 2026-11-03 | the buy that gives PW 5 SECW           |

The `holdings-*.json` files are `SyntheticHoldings` snapshots for the CUR-021
holdings tests. Brokerage account X has quantity accounts Q (SECA, ID ending
`6100`) and QB (SECB, `6150`); brokerage account Y has QY (SECA, `6160`).
Identifier `SYN-0001` names SECA and `SYN-9999` names no security.

| File                                  | Scope | Completeness | As of      | Positions            |
| ------------------------------------- | ----- | ------------ | ---------- | -------------------- |
| `holdings-x-partial.json`             | X     | Partial      | 2026-11-30 | SECA 6               |
| `holdings-x-complete-2026-11-30.json` | X     | Complete     | 2026-11-30 | SECA 6               |
| `holdings-x-complete-2026-12-31.json` | X     | Complete     | 2026-12-31 | SECA 6               |
| `holdings-y-complete.json`            | Y     | Complete     | 2026-11-30 | SECA 2               |
| `holdings-x-unmapped.json`            | X     | Partial      | 2026-11-30 | SECA 6, SYN-9999 5   |
| `holdings-x-complete-unmapped.json`   | X     | Complete     | 2026-11-30 | SECA 6, SYN-9999 3   |
