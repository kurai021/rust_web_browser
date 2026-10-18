// Copyright (C) 2018 Rick Waldron. All rights reserved.
// BSD-3-Clause; ../LICENSE. Upstream: built-ins/Promise/all/iter-arg-is-string-resolve.js
try {
  Promise.all("").then(function(v) { assert.sameValue(v.length, 0); }, function() {
    $DONE('The promise should be resolved, but was rejected');
  }).then($DONE, $DONE);
} catch (error) {
  $DONE(`The promise should be resolved, but threw an exception: ${error.message}`);
}
