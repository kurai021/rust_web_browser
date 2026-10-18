// Copyright 2016 Microsoft, Inc. All rights reserved.
// BSD-3-Clause; ../LICENSE. Upstream: language/statements/async-function/evaluation-body-that-throws-after-await.js
async function foo() { await Promise.resolve(); throw 1; }
foo().then(function() { $DONE("Should not be called"); }, function(e) {
  assert.sameValue(e, 1); $DONE();
});
