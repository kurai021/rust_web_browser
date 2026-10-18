// Copyright (C) 2015 the V8 project authors. All rights reserved.
// BSD-3-Clause; ../LICENSE. Upstream: built-ins/Promise/resolve/arg-poisoned-then.js
var poisonedThen = {};
var err = new Test262Error();
Object.defineProperty(poisonedThen, 'then', { get: function() { throw err; } });
Promise.resolve(poisonedThen).then(function() {
  throw new Test262Error('Promise should be rejected when retrieving `then` property throws an error');
}, function(reason) { assert.sameValue(reason, err); }).then($DONE, $DONE);
