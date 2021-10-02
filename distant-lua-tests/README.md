# Tests for Distant Lua (module)

Contains tests for the **distant-lua** module. These tests must be in a
separate crate due to linking restrictions as described in 
[khvzak/mlua#79](https://github.com/khvzak/mlua/issues/79).

## Tests

You must run these tests from within this directory, not from the root of the
repository. Additionally, you must build the Lua module **before** running
these tests!

```bash
# From root of repository
(cd distant-lua && cargo build --release)
```

Running the tests themselves:

```bash
# From root of repository
(cd distant-lua-tests && cargo test --release)
```

## License

This project is licensed under either of

Apache License, Version 2.0, (LICENSE-APACHE or
[apache-license][apache-license]) MIT license (LICENSE-MIT or
[mit-license][mit-license]) at your option.

[apache-license]: http://www.apache.org/licenses/LICENSE-2.0
[mit-license]: http://opensource.org/licenses/MIT
