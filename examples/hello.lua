-- Minimal round-trip check.
-- Paste into the Execute panel after attaching, or:
--   solara exec <pid> examples/hello.lua
print("solara attached")

local t = {
    pid = "see the target chip in the ui",
    time = os.time and os.date and os.date("%H:%M:%S") or "n/a",
}

for k, v in pairs(t) do
    print(string.format("  %-6s %s", k, tostring(v)))
end

return true
