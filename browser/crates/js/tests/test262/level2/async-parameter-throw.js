// Copyright 2016 Microsoft, Inc. All rights reserved.
// BSD-3-Clause; ../LICENSE. Upstream: language/statements/async-function/evaluation-default-that-throws.js
var y = null;
async function foo(x = y()) {}
foo().then(function() { $DONE('promise should be rejected'); }, function() { $DONE(); });
