--!nonstrict
-- The control-center tile. Runs after stubs.luau and shortcut.luau.

onClick()
assert(
  calls.stream[1] == "(noctalia msg panel-close >/dev/null 2>&1; sleep 0.3 && valw region) 2>&1; echo valw-exit:$?",
  calls.stream[1]
)
assert(#calls.run == 0, "never through runAsync's timeout")
reset()
fake.streams["(noctalia msg panel-close"] = valwFailure("busy", 1)
onClick()
assert(calls.notify[1][2] == "busy", "failure notified")
reset()
fake.streams["(noctalia msg panel-close"] = { "valw-exit:3" }
onClick()
assert(#calls.notify == 0, "cancel is silent")
ok("click")

reset()
onRightClick()
assert(calls.toggled[1] == "elars/valw:toolbar", "right click toggles the panel")
ok("right click")
