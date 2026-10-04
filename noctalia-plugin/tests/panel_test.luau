--!nonstrict
-- The toolbar panel. Runs after stubs.luau and panel.luau.

local STATE = { mode = "zoom", timer = 5, cursor = false, preview = true, sound = true, timers = { 0, 5, 10 } }

-- valw missing from Noctalia's PATH: a label and a notification, no panel.
fake.exists = false
onOpen(nil)
local labels = findAll(rendered, function(n) return n.kind == "label" end)
assert(#labels == 1 and labels[1].props.text == "panel.unavailable", "unavailable label")
assert(#calls.notify == 1, "notified once")
assert(#calls.run == 0, "nothing run")
ok("valw missing")

-- --state failing: the same.
reset()
fake.exists = true
fake.answers["valw toolbar --state"] = { exitCode = 1, stdout = "", stderr = "warn\nerror: broken\n" }
onOpen(nil)
assert(findAll(rendered, function(n) return n.kind == "label" and n.props.text == "panel.unavailable" end)[1], "label")
assert(calls.notify[1][2] == "error: broken", "the stderr's last line: " .. tostring(calls.notify[1][2]))
ok("--state failing")

-- A working valw: the remembered mode is primary, the timer selected.
reset()
fake.answers["valw toolbar --state"] = { exitCode = 0, stdout = "{}", stderr = "" }
fake.decoded = STATE
onOpen(nil)
assert(calls.run[1] == "valw toolbar --state", calls.run[1])
assert(button("mode.zoom").props.variant == "primary", "zoom is primary")
for _, m in ipairs({ "mode.screen", "mode.region", "mode.window" }) do
  assert(button(m).props.variant == "outline", m .. " is outline")
end
local select = findAll(rendered, function(n) return n.kind == "select" end)[1]
assert(select.props.selectedIndex == 1, "5 s is index 1 (0-based)")
assert(select.props.options[1] == "timer.off" and select.props.options[2] == "5 s", "timer labels")
local toggles = findAll(rendered, function(n) return n.kind == "toggle" end)
assert(#toggles == 3, "three toggles")
assert(toggles[1].props.checked == false and toggles[2].props.checked == true, "cursor off, preview on")
local hex = findAll(rendered, function(n)
  for _, v in pairs(n.props) do
    if type(v) == "string" and string.find(v, "^#") then return true end
  end
  return false
end)
assert(#hex == 0, "no hex colours")
ok("renders the state")

-- A mode click closes the panel first, then captures after the wait.
reset()
button("mode.region").props.onClick()
assert(calls.order[1] == "close" and calls.order[2] == "run", "close, then run")
assert(calls.run[1] == "sleep 0.3 && valw toolbar --run region", calls.run[1])
assert(#calls.notify == 0, "success is silent")
ok("mode click")

-- A failed capture notifies; a cancelled one doesn't.
reset()
fake.answers["sleep 0.3 && valw toolbar --run"] = { exitCode = 1, stdout = "", stderr = "error: no outputs\n" }
button("mode.screen").props.onClick()
assert(calls.notify[1][2] == "error: no outputs", "failure notified")
reset()
fake.answers["sleep 0.3 && valw toolbar --run"] = { exitCode = 3, stdout = "", stderr = "" }
button("mode.window").props.onClick()
assert(#calls.notify == 0, "cancel is silent")
ok("capture results")

-- The timer and the toggles write through --set and re-render at once.
reset()
fake.answers["sleep 0.3 && valw toolbar --run"] = nil
select = findAll(rendered, function(n) return n.kind == "select" end)[1]
select.props.onChange("2")
assert(calls.run[1] == "valw toolbar --set timer=10", calls.run[1])
select = findAll(rendered, function(n) return n.kind == "select" end)[1]
assert(select.props.selectedIndex == 2, "re-rendered with 10 s")
reset()
toggles = findAll(rendered, function(n) return n.kind == "toggle" end)
toggles[3].props.onChange("false")
assert(calls.run[1] == "valw toolbar --set sound=false", calls.run[1])
toggles = findAll(rendered, function(n) return n.kind == "toggle" end)
assert(toggles[3].props.checked == false, "re-rendered with sound off")
reset()
fake.answers["valw toolbar --set"] = { exitCode = 1, stdout = "", stderr = "error: read-only\n" }
toggles[1].props.onChange("true")
assert(calls.run[1] == "valw toolbar --set cursor=true", calls.run[1])
assert(calls.notify[1][2] == "error: read-only", "a failed --set notifies")
ok("timer and toggles")
