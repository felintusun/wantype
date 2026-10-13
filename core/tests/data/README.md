# Test data

`mozc-romanji-hiragana.tsv`: the romaji → hiragana table of
[Mozc](https://github.com/google/mozc) (Google Japanese Input), at commit
`60af02ff797275f2ba1b7fddccdec916798d112e` (`src/data/preedit/romanji-hiragana.tsv`). BSD 3-Clause, see
`MOZC-LICENSE`. The tests use it as the reference for standard romaji input.
Columns: typed keys, output, and keys carried over to the next output
(`kk` → っ, then `k`).
