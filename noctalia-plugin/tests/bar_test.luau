--!nonstrict
-- The bar widget. Runs after stubs.luau and bar.luau.

update()
assert(findAll(rendered, function(n) return n.kind == "glyph" and n.props.name == "camera" end)[1], "camera glyph")
ok("renders")

onClick()
assert(calls.toggled[1] == "elars/valw:toolbar", "left click toggles the panel")
assert(#calls.run == 0)
ok("left click")

reset()
onRightClick()
assert(calls.run[1] == "valw region", calls.run[1])
reset()
fake.answers["valw region"] = { exitCode = 1, stdout = "", stderr = "error: busy\n" }
onRightClick()
assert(calls.notify[1][2] == "error: busy", "failure notified")
reset()
fake.answers["valw region"] = { exitCode = 3, stdout = "", stderr = "" }
onRightClick()
assert(#calls.notify == 0, "cancel is silent")
ok("right click")
