-- A typical client-side inspection script.
-- This is the shape of what an executor script looks like: it walks the
-- exposed services, lists what is reachable, and reports without mutating
-- anything. Paste into the Execute panel, or:
--   solara exec <pid> examples/inspect.lua

local services = {
    "Players",
    "Workspace",
    "ReplicatedStorage",
    "Lighting",
    "RunService",
    "UserInputService",
}

local function describe(name)
    local ok, svc = pcall(function()
        return game:GetService(name)
    end)
    if not ok or not svc then
        return string.format("%-20s unavailable", name)
    end
    local kids = 0
    pcall(function()
        kids = #svc:GetChildren()
    end)
    return string.format("%-20s %s (%d children)", name, svc.ClassName, kids)
end

print("== services ==")
for _, name in ipairs(services) do
    print("  " .. describe(name))
end

print("== local player ==")
local players = game:GetService("Players")
local lp = players and players.LocalPlayer
if lp then
    print("  name     " .. tostring(lp.Name))
    print("  userid   " .. tostring(lp.UserId))
    print("  team     " .. tostring(lp.Team and lp.Team.Name or "none"))
else
    print("  no LocalPlayer yet (still loading?)")
end

return true
