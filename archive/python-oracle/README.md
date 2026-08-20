# Python oracle (archived)

This is the **original Python implementation** of NeoBrowser. It is **not the product**
and is not maintained.

The product is the Rust binary in [`rust/`](../../rust/).

## Why it is still here

It was the reference implementation while the Rust port was catching up, and it remains a
useful record of *how* several non-obvious things work:

- `cookie_sync.py` — cross-platform Chrome cookie decryption (Keychain / secret-service /
  DPAPI), including the AES-CBC vs AES-GCM split and the PBKDF2 iteration counts.
- `page_analyzer.py` — the layered element-finding heuristics that the Rust `find` and
  `observe` grew out of.
- `security.py`, `perception.py` — earlier takes on problems the Rust side now solves
  differently.

## Why it was archived

Two concrete reasons, not tidiness.

**It had stopped being an oracle.** Differential testing against it only works while both
implementations agree on what a tool returns. They no longer do: the Rust mutating tools
return a verified-action envelope (`status`, `evidence`, `warnings`) instead of a
confirmation string, and no version of `scripts/compare.py` could pass without reverting
that. Keeping a parity gate that must fail is worse than not having one.

**Its packaging was actively misleading.** `pyproject.toml` declared `version = "1.0.0"`
while the shipped product was `0.1.7`. Anyone running `pip install neobrowser` from the
repository root would have installed the implementation that is *not* the product, under a
version number implying it was the mature one.

## Running it, if you need to

```bash
cd archive/python-oracle
pip install -e ".[dev]"
python -m pytest -q
```

The tests still pass. They test the Python implementation's own behaviour, so a failure
here says nothing about the Rust product.
