// Copyright 2016 Microsoft, Inc. All rights reserved.
// BSD-3-Clause; ../LICENSE. Upstream: language/statements/async-function/evaluation-body-that-returns-after-await.js
async function foo() { await Promise.resolve(); return 42; }
foo().then(function(v) { assert.sameValue(v, 42); $DONE(); }, $DONE);
