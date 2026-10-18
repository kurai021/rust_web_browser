// Copyright (C) 2016 the V8 project authors. All rights reserved.
// BSD-3-Clause; ../LICENSE. Upstream: built-ins/Promise/resolve/resolve-thenable.js
var value = {};
var thenable = { then: function(resolve) { resolve(value); } };
Promise.resolve(thenable).then(function(val) {
  if (val !== value) { $DONE('The promise should be fulfilled with the provided value.'); return; }
  $DONE();
}, function() { $DONE('The promise should not be rejected.'); });
