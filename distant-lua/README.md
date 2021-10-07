# Distant Lua (module)

Contains the Lua module wrapper around several distant libraries
including:

1. **distant-core**
2. **distant-ssh2**

## Building

*Compilation MUST be done within this directory! This crate depends on
.cargo/config.toml settings, which are only used when built from within this
directory.*

```bash
# Outputs a library file (*.so for Linux, *.dylib for MacOS)
cargo build --release
```

## Examples

Rename `libdistant_lua.so` or `libdistant_lua.dylib` to `distant_lua.so`
(yes, **.so** for **.dylib**) and place the library in your Lua path.

```lua
local distant = require("distant_lua")

-- Distant functions are async by design and need to be wrapped in a coroutine
-- in order to be used
local thread = coroutine.wrap(distant.launch)

-- Initialize the thread
thread({ host = "127.0.0.1" })

-- Continually check if launch has completed
local res
while true do
    res = thread()
    if res ~= distant.PENDING then
        break
    end
end
```

## Tests

Tests are run in a separate crate due to linking described here:
[khvzak/mlua#79](https://github.com/khvzak/mlua/issues/79). You **must** build
this module prior to running the tests!

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
