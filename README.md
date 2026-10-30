# solara

Linux process toolkit for Roblox clients (Sober / Vinegar / Wine).
Attach to the client, read and write its memory, inject a shared object,
and drive it from a browser control panel or the CLI.

Built for Arch Linux, plain Rust, one dependency (`libc`).

---

## Quick start

```sh
cargo build --release

sudo sysctl kernel.yama.ptrace_scope=0   # see "The permission gate" below
launch your Roblox client (Sober)

./target/release/solara ui               # ← opens the control panel
```

Then in the panel: click a target → **Attach** → type a script → **Execute**.

Prefer the terminal? Same thing:

```sh
./target/release/solara find
./target/release/solara inject <pid> target/release/libpayload.so
./target/release/solara exec <pid> "print('hi')"
```

Note: the shipped payload round-trips scripts through the injected thread — it
proves attach + injection + IPC. It does **not** yet evaluate Luau inside the
game. See [Status](#status).

---

## Open the UI

```sh
./target/release/solara ui
```

```
solara ui  http://127.0.0.1:8787
ctrl-c to stop
```

It binds `127.0.0.1` only and opens your browser automatically (`xdg-open`).
If the port is busy or you want another one:

```sh
./target/release/solara ui 6799     # → http://127.0.0.1:6799
```

Everything the UI does is a local HTTP call to the same binary — there is no
separate frontend build, no node, no webkit. The whole page is embedded in the
binary with `include_str!`.

### Panel layout

```
┌────────────────────────────────────────────────────────────────────┐
│ SOLARA.  linux roblox toolkit          ● 1 target       [Refresh]  │
├─────────────────────────────────┬──────────────────────────────────┤
│ TARGETS                     1   │ EXECUTE           pid 158997     │
│                                 │ ┌──────────────────────────────┐ │
│ pid     process   ns     flat…  │ │ print("hello from solara")   │ │
│ 158997  sober     shared   -    │ └──────────────────────────────┘ │
│                       [Attach]  │ [Execute]  ?         ctrl+enter  │
│                                 ├──────────────────────────────────┤
│                                 │ MEMORY                           │
│                                 │ addr  [0x7f2a…]   len [64]       │
│                                 │ [Read]                           │
│                                 │ pattern [7f454c46]               │
│                                 │ [Scan]                           │
├─────────────────────────────────┴──────────────────────────────────┤
│ OUTPUT                                                   [Clear]   │
│   mmap    @ 0x7fe21e51c370                                         │
│   dlopen  @ 0x7fe21e494580                                         │
│   staged  -> /proc/158997/root/tmp/libpayload.so                   │
│   ok      attached to pid 158997                                   │
└────────────────────────────────────────────────────────────────────┘
```

### Walkthrough

**1. Find the client.** The Targets table refreshes every 5 seconds on its own,
or hit **Refresh**. Each row shows:

| column | meaning |
|---|---|
| `pid` | process id |
| `process` | executable name, with the full cmdline on hover |
| `ns` | `shared` = same namespace as solara, `isolated` = inside a sandbox |
| `flatpak` | Flatpak app id when the target runs under Flatpak |

`isolated` is highlighted amber — that means the client lives in a mount
namespace you are not in, and the sandbox rules in
[Running against Sober](#running-against-sober) apply.

**2. Select it.** Click anywhere on the row (except the button). The row gets a
teal edge and the target chip in the Execute panel fills in with the pid.

**3. Attach.** Press the row's **Attach** button. The Output pane shows the
whole injection as it happens:

```
  mmap    @ 0x7fe21e51c370
  dlopen  @ 0x7fe21e494580
  dlsym   @ 0x7fe21e494640
  staged  -> /proc/158997/root/tmp/libpayload.so
  alloc   @ 0x7fe21e623000
  loaded  handle=0x557e764d0420
  solara_payload_start @ 0x7fe21e3b7ef0
  started
ok     attached to pid 158997
```

On success the row flips to a green `attached` badge and the Execute / Read /
Scan buttons unlock. On failure the trace stops at the failing step and prints
the reason in red — most commonly `Operation not permitted`, which is the
permission gate below.

**4. Execute.** Type a script and press **Execute** (or `Ctrl+Enter`). The
payload answers on its per-pid abstract socket:

```
[solara-payload] pid=158997 received 24 bytes:
print('executed via ui')
--
note: luau vm hook not connected. this confirms injection + ipc only.
```

**5. Poke memory.** In the Memory panel, `addr` is hex (`0x` optional), `len`
is decimal and capped at 8192 bytes:

```
addr    0x7ffeccb18458
len     32
[Read]
```

```
00007ffeccb18458  7f 45 4c 46 02 01 01 00 00 00 00 00 00 00 00 00  |.ELF............|
00007ffeccb18468  03 00 3e 00 01 00 00 00 00 00 00 00 00 00 00 00  |..>.............|
```

`pattern` takes hex bytes. **Scan** walks the writable regions and lists every
hit:

```
7f454c46
[Scan]
```

```
1 hit(s)
0x7ffeccb18458
```

Hit count and addresses print in Output; copy one into `addr` and hit Read to
look at it. If the process cannot be read at all, Scan now reports the real
error instead of a misleading `no hits`.

**6. Output.** Everything above lands in the Output pane, colour-coded:
teal for actions you triggered, green for success, red for errors, dim for
steps. **Clear** wipes it. The pane keeps the last 2000 lines.

---

## Requirements

| need | why |
|---|---|
| Linux x86_64 | ptrace + SysV register layout |
| Rust 1.70+ | `from_abstract_name`, `Mutex::new` in const |
| `ptrace` permission | every read/write/attach goes through it |
| `xdg-open` | optional, only to auto-open the browser |

No GTK, no Qt, no WebKit, no Node.

---

## Build

```sh
cargo build --release
```

Produces two artifacts:

```
target/release/solara            the CLI + UI server
target/release/libpayload.so     the injected shared object
```

The `libpayload.so` must sit where `solara` can find it — the release dir is
one of the probed locations, so the command above is enough. If you move the
binary elsewhere, use `solara inject <pid> /full/path/libpayload.so`.

`cargo build` (debug) also works; the payload is loaded from `target/debug/`.

---

## The permission gate

Linux refuses to let one process inspect another unless it is allowed to.
Arch ships `kernel.yama.ptrace_scope = 1`, which means **only a parent may
attach to a child**. That is why attaching to a client you launched yourself
fails with `Operation not permitted` even though both run as you.

Pick one:

```sh
# session-wide, resets on reboot
sudo sysctl kernel.yama.ptrace_scope=0

# permanent
echo 'kernel.yama.ptrace_scope = 0' | sudo tee /etc/sysctl.d/10-ptrace.conf

# or just elevate the tool
sudo ./target/release/solara ui
```

Verify:

```sh
cat /proc/sys/kernel/yama/ptrace_scope    # want 0
```

This is the single most common reason attach fails. Every other failure mode
is in [Troubleshooting](#troubleshooting).

---

## CLI reference

```sh
solara ui [port]                    # browser control panel (default 8787)

solara find                         # list targets + namespace status
solara maps <pid> [filter]          # memory map, optional path filter
solara read <pid> <addr> <len>      # hexdump
solara write <pid> <addr> <hex>     # write bytes
solara scan <pid> <hex>             # scan writable regions

solara inject <pid> <path.so>       # ptrace + dlopen + dlsym + call
solara exec <pid> <script|file>     # send a script to the payload
solara inject-test [--hold]         # end-to-end attach test on a dummy

solara sober info <pid>             # namespace + flatpak id
solara sober enter <pid> [cmd]      # shell inside the target's namespaces

solara selftest                     # attach/read/write/scan on a child
```

Addresses and byte patterns are hex, `0x` optional. `exec` treats its argument
as a file when one exists, otherwise as an inline script.

Examples:

```sh
solara find
solara maps 158997 libc.so           # just libc
solara read 158997 0x7ffeccb18458 64
solara scan 158997 7f454c46
solara write 158997 0x55d0 90909090
solara exec 158997 script.lua
```

---

## Running against Sober

Sober runs the client inside a Flatpak / bubblewrap sandbox. Two things break
if you ignore it, and solara handles both.

**Mount namespace.** `dlopen` executes inside the *target's* filesystem, so a
path on your host may not exist there. `inject` copies the `.so` to
`/proc/<pid>/root/tmp/` and passes `/tmp/libpayload.so` — the target's own
`/tmp` — so the path always resolves:

```
staged  -> /proc/158997/root/tmp/libpayload.so
```

**Private `/tmp`.** The IPC socket is an **abstract** Unix socket named
`solara-exec-<pid>`, not a filesystem path, so it is reachable across mount
namespaces from either side.

Walkthrough:

```sh
solara find                          # 1. get the game pid
solara sober info <pid>              # 2. check same_mnt / flatpak
```

```
pid_ns:    pid:[4026531836]
mnt_ns:    mnt:[4026532199]
same_pid:  true
same_mnt:  false              ← sandboxed, keep reading
flatpak:   org.vinegarhq.Sober
```

If `same_mnt` is `true` you can attach normally. If it is `false`, the `.so`
staging still works but you need elevated rights to reach into the sandbox:

```sh
sudo solara inject <pid> target/release/libpayload.so
solara exec <pid> "print('hi')"
```

Or go inside the sandbox and work from there:

```sh
sudo solara sober enter <pid>
# now in the client's namespaces:
ip addr
ls /tmp
```

`solara sober enter` does `setns(mnt)` → `setns(pid)` → `fork` → `execvp`, so
the child really is inside the target's mount and PID namespaces.

---

## How attach works

`inject` is seven steps, each one logged to the Output pane:

1. `dladdr` on a local `dlsym` result gives the library load base and filename;
   the same library is located in `/proc/<pid>/maps` to compute its base there.
   `remote = remote_base + (local_addr - local_base)` resolves `mmap`, `dlopen`
   and `dlsym` inside the target.
2. `PTRACE_ATTACH` — target stops.
3. `call_remote(mmap, ...)` allocates one 4 KiB page inside the target.
4. The `.so` path is written into that page with `process_vm_writev`.
5. `call_remote(dlopen, path, RTLD_NOW)` — the loader maps the `.so` and runs
   its constructors.
6. `call_remote(dlsym, handle, "solara_payload_start")` finds the entry point.
7. `call_remote(entry)` — the payload spawns a thread that binds
   `@solara-exec-<pid>` and serves script requests.

`call_remote` is the interesting part. It cannot just jump into a function and
hope for a return, so it builds a frame:

- save every register, remember the byte at `rip`
- write `0xCC` (int3) at the saved `rip` — that becomes the return target
- move `rsp` 2 KiB down, 16-byte align, and write the saved `rip` there as the
  return address (SysV wants `rsp ≡ 8 mod 16` at function entry)
- set `rip` to the function, fill `rdi rsi rdx rcx r8 r9`, `PTRACE_CONT`
- the function runs, `ret` pops our address, hits `0xCC`, the kernel delivers
  `SIGTRAP` and we read `rax`
- restore the original byte and every saved register

Code pages are `r-xp`, so the breakpoint is written with `PTRACE_POKETEXT`,
which bypasses page protection, while reads use `process_vm_readv`.

---

## Troubleshooting

| symptom | cause | fix |
|---|---|---|
| `Operation not permitted` on attach/read | `ptrace_scope=1` and target is not your child | `sudo sysctl kernel.yama.ptrace_scope=0` or run as root |
| `no roblox / sober / wine process found` | client not running, or name differs | launch the client, hit Refresh |
| UI says `bind 127.0.0.1:8787: Address already in use` | port taken | `solara ui 6799` |
| `dlopen returned NULL` | `.so` path not visible in the target's mount namespace | check `staged ->` line; if staging was skipped, run as root |
| `stage skipped (Permission denied)` | cannot write to `/proc/<pid>/root/tmp` | run as root, or `solara sober enter` first |
| `connect @solara-exec-<pid>: Connection refused` | payload not injected, or the process was restarted | re-attach |
| `expected SIGTRAP, got signal …` | the called function did not return normally | rare; re-attach and retry |
| Scan prints nothing useful | process unreadable | check the ptrace gate, then rescan |

Two self-checks that need no Roblox client at all:

```sh
solara selftest      # spawns a child, proves attach/read/write/scan
solara inject-test   # spawns a child, proves inject + IPC end to end
```

Both should print `PASS`. If they do, any remaining failure is permissions or
namespaces, not the toolkit.

---

## Project layout

```
src/main.rs            CLI entry, command dispatch, self-tests
src/mem.rs             process_vm_readv/writev, /proc maps, hexdump, scanner
src/ptrace.rs          attach, get/setregs, call_remote (int3 technique)
src/inject.rs          symbol resolution, .so staging, dlopen/dlsym pipeline
src/sober.rs           namespace detection, setns+fork, flatpak ids
src/ui.rs              local HTTP server + JSON-free API
src/ui_assets/index.html   the panel (embedded via include_str!)
src/trace.rs           shared log buffer (terminal and UI see the same trace)
payload/src/lib.rs     the injected .so: exports solara_payload_start
```

---

## Status

Working and verified end to end:

- attach, detach, register access
- remote memory read (`process_vm_readv`), write (`process_vm_writev`, `PTRACE_POKETEXT`)
- writable-region pattern scanner
- arbitrary function calls inside the target (`call_remote`)
- shared-object injection via `dlopen` + `dlsym`
- cross-namespace `.so` staging and abstract-socket IPC
- browser control panel and full CLI

Not implemented: the Luau VM hook. `exec` round-trips the script through the
injected payload and echoes it back. It proves the pipeline works; it does not
evaluate code in the game. That step needs the client's Luau `lua_State` and a
call into `luaL_loadbuffer` / `lua_pcall`, which sits behind Hyperion (Roblox's
anti-tamper). No public Linux bypass exists.
