--!nonstrict
-- The bar widget. Runs after stubs.luau and bar.luau.

update()
assert(findAll(rendered, function(n) return n.kind == "glyph" and n.props.name == "camera" end)[1], "camera glyph")
ok("renders")

onClick()
assert(calls.toggled[1] == "elars/valw:toolbar", "left click toggles the panel")
assert(#calls.run == 0 and #calls.stream == 0)
ok("left click")

reset()
onRightClick()
assert(calls.stream[1] == "(valw region) 2>&1; echo valw-exit:$?", calls.stream[1])
assert(#calls.run == 0, "never through runAsync's timeout")
reset()
fake.streams["(valw region)"] = valwFailure("busy", 1)
onRightClick()
assert(calls.notify[1][2] == "busy", "failure notified")
reset()
fake.streams["(valw region)"] = { "valw-exit:3" }
onRightClick()
assert(#calls.notify == 0, "cancel is silent")
ok("right click")
