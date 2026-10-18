// Copyright 2017 Caitlin Potter. All rights reserved.
// BSD-3-Clause; ../LICENSE. Upstream: language/statements/async-function/try-return-finally-throw.js
async function f() {
  try { return "early-return"; } finally { throw "override"; }
}
f().then($DONE, function(value) {
  assert.sameValue(value, "override", "Exception thrown in finally block");
}).then($DONE, $DONE);
