# solara

Linux process toolkit for Roblox clients (Sober / Vinegar / Wine). Attach,
read/write memory, inject a shared object, and talk to it over a socket.

## Build

```sh
cargo build --release
```

Produces:
- `target/release/solara` — the CLI
- `target/release/libpayload.so` — the injected shared object

## UI

```sh
solara ui            # http://127.0.0.1:8787, opens in your browser
solara ui 6799       # custom port
```

A single-page control panel served from the binary — no GTK/WebKit, no extra
dependencies. It talks to `127.0.0.1` only.

- **Targets** — auto-refreshes every 5 s; click a row to select, `Attach` injects
- **Execute** — script box, `Execute` button (or `Ctrl+Enter`)
- **Memory** — read at an address, scan for a hex byte pattern
- **Output** — full injection trace (`mmap @ …`, `dlopen @ …`, `staged →`, …)

If attach fails with `Operation not permitted`, the panel prints the trace up to
the failure and the ptrace hint. Start it with `sudo` or enable ptrace for the
session first.

## Attach

```sh
solara find                      # list roblox/sober/wine processes
solara maps <pid> [filter]       # memory map
solara read <pid> <addr> <len>   # hexdump
solara write <pid> <addr> <hex>  # write bytes
solara scan <pid> <hex>          # scan writable regions
solara selftest                  # verify attach/read/write/scan
```

`ptrace_scope=1` (Arch default) only allows attaching to child processes.
For anything else:

```sh
sudo sysctl kernel.yama.ptrace_scope=0
# or run solara as root
```

## Execute

```sh
solara inject <pid> target/release/libpayload.so
solara exec <pid> "print('hello')"     # inline script
solara exec <pid> script.lua           # from file
solara inject-test                     # end-to-end self-test
```

`inject` does:

1. `dladdr` + `/proc/pid/maps` → resolve `mmap` / `dlopen` / `dlsym` in the target
2. `PTRACE_ATTACH`
3. `call_remote(mmap, ...)` → allocate a 4 KiB page in the target
4. write the `.so` path into that page
5. `call_remote(dlopen, path, RTLD_NOW)` → load the `.so`
6. `call_remote(dlsym, handle, "solara_payload_start")`
7. `call_remote(fn, ...)` → payload spawns its IPC thread

`call_remote` builds a fake stack frame 2 KiB below the target's `rsp`,
writes the return address there, sets `rip` to the function and the SysV
argument registers, then catches the return via an `int3` breakpoint at the
original `rip`.

## Sober

Sober runs the client inside a Flatpak/bwrap sandbox. Two things break naïvely:

**1. Mount namespace.** `dlopen` runs in the *target's* filesystem. The
injector copies the `.so` to `/proc/<pid>/root/tmp/` and passes
`/tmp/libpayload.so`, so the path resolves inside the sandbox.

**2. Private `/tmp`.** The IPC socket is an **abstract** Unix socket
(`solara-exec-<pid>`), not a filesystem path, so it crosses mount namespaces.

Workflow:

```sh
solara find                        # grab the game pid
solara sober info <pid>            # check same_pid / same_mnt / flatpak id

# if same_mnt: false, you must be in the target's mount namespace:
sudo solara inject <pid> target/release/libpayload.so
# or:
sudo solara sober enter <pid>      # shell inside the sandbox
#   then: solara inject <pid> /tmp/libpayload.so

solara exec <pid> "print('hi')"
```

`solara sober enter <pid> [cmd]` does `setns(mnt)` + `setns(pid)` + `fork` +
`execvp`, so the child sees the target's filesystem and PID namespace.

## Status

Working and tested: attach, memory R/W, scanner, `.so` injection, IPC,
Sober namespace handling.

Not implemented: the Luau VM hook. `exec` currently round-trips the script
through the payload and echoes it back — it proves injection and IPC, it does
**not** run code in the game. That requires locating the Luau `lua_State` in
the client and calling `luaL_loadbuffer`/`lua_pcall`, which is gated behind
Hyperion (Roblox's anti-tamper). No public Linux bypass exists.
