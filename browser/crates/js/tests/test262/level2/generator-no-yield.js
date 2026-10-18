// Copyright (C) 2013 the V8 project authors. All rights reserved.
// BSD-3-Clause; ../LICENSE. Upstream: language/statements/generators/no-yield.js
function *foo(a) {}
var g = foo(3);
assert.sameValue(g.next().value, undefined);
assert.sameValue(g.next().done, true);
