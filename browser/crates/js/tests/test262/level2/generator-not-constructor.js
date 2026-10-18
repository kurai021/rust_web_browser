// Copyright (C) 2015 André Bargull. All rights reserved.
// BSD-3-Clause; ../LICENSE. Upstream: language/statements/generators/invoke-as-constructor.js
function* g(){}
assert.throws(TypeError, function() { var instance = new g(); });
