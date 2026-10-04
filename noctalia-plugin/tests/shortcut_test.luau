--!nonstrict
-- The control-center tile. Runs after stubs.luau and shortcut.luau.

onClick()
assert(calls.run[1] == "noctalia msg panel-close; sleep 0.3 && valw region", calls.run[1])
reset()
fake.answers["noctalia msg panel-close"] = { exitCode = 1, stdout = "", stderr = "error: busy\n" }
onClick()
assert(calls.notify[1][2] == "error: busy", "failure notified")
reset()
fake.answers["noctalia msg panel-close"] = { exitCode = 3, stdout = "", stderr = "" }
onClick()
assert(#calls.notify == 0, "cancel is silent")
ok("click")

reset()
onRightClick()
assert(calls.toggled[1] == "elars/valw:toolbar", "right click toggles the panel")
ok("right click")
