# openCypher TCK

This directory vendors the official openCypher Technology Compatibility Kit
(TCK), release **2024.3** (opencypher/openCypher commit
`677cbafabb8c3c5eed458fd3b1ec0daec8d67d23`, recorded in `UPSTREAM.toml`):

- `features/`: the Gherkin scenarios, copied unmodified;
- `graphs/`: the Cypher scripts of the named graphs some scenarios start from.

The TCK content is licensed under the Apache License 2.0 by Neo4j Sweden AB;
see `LICENSE` and `NOTICE` in this directory. These files are not covered by
the crate's own licenses and are excluded from the published crate.

Unlike decypher's syntax-only TCK gate, `tests/tck.rs` *executes* every
scenario against a petgraph graph: it runs the setup queries, executes the
query under test, and compares the result table, the expected errors and the
side effects.

`known_failures.txt` lists the scenarios that currently fail. The test fails on
any failure not in the list and on any listed scenario that now passes, so the
list can only shrink. Regenerate it with:

```sh
TCK_BLESS=1 cargo test --test tck
```

Filter scenarios with `TCK_FILTER=<substring of the feature path or name>` and
print every failure with `TCK_VERBOSE=1`.
