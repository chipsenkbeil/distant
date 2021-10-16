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
# Outputs a library file (*.so for Linux, *.dylib for MacOS, *.dll for Windows)
cargo build --release
```

## Examples

Rename `libdistant_lua.so` or `libdistant_lua.dylib` to `distant_lua.so`
(yes, **.so** for **.dylib**) and place the library in your Lua path at
the *root*. The library cannot be within any submodule otherwise it fails
to load appropriate symbols. For neovim, this means directly within the
`lua/` directory.

```lua
local distant = require("distant_lua")

-- The majority of the distant lua module provides async and sync variants
-- of methods; however, launching a session is currently only synchronous
local session = distant.session.launch({ host = "127.0.0.1" })

-- Sync methods are executed in a blocking fashion, returning the result of
-- the operation if successful or throwing an error if failing. Use `pcall`
-- if you want to capture the error
local success, result = pcall(session.read_dir, session, { path = "path/to/dir" })
if success then
    for _, entry in ipairs(result.entries) do
        print("Entry", entry.file_type, entry.path, entry.depth)
    end
else
    print(result)
end

-- Async methods have _async as a suffix and need to be polled from
-- Lua in some manner; the `wrap_async` function provides a convience
-- to do so taking an async distant function and a scheduling function
local schedule_fn = function(cb) end
local read_dir = distant.utils.wrap_async(session.read_dir_async, schedule_fn)
read_dir(session, { path = "path/to/dir" }, function(success, result)
    -- success: Returns true if ok and false if err
    -- result: If success is true, then is the resulting value,
    --         otherwise is the error
    print("Success", success)
    if success then
        for _, entry in ipairs(result.entries) do
            print("Entry", entry.file_type, entry.path, entry.depth)
        end
    else
        print(result)
    end
end)

-- For neovim, there exists a helper function that converts async functions
-- into functions that take callbacks, executing the asynchronous logic
-- using neovim's event loop powered by libuv
local read_dir = distant.utils.nvim_wrap_async(session.read_dir_async)
read_dir(session, { path = "path/to/dir" }, function(success, result)
    -- success: Returns true if ok and false if err
    -- result: If success is true, then is the resulting value,
    --         otherwise is the error
    print("Success", success)
    if success then
        for _, entry in ipairs(result.entries) do
            print("Entry", entry.file_type, entry.path, entry.depth)
        end
    else
        print(result)
    end
end)
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
