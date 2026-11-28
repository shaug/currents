# Synthetic trade inputs

The CUR-021 trade tests in `crates/currents-domain/tests/cur_021.rs` import
these files. Every account, security and amount in them is invented.

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
