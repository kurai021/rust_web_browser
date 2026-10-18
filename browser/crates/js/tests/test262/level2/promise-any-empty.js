// Copyright (C) 2019 Sergey Rubanov. All rights reserved.
// BSD-3-Clause; ../LICENSE. Upstream: built-ins/Promise/any/iter-arg-is-empty-iterable-reject.js
Promise.any([]).then(() => $DONE('The promise should be rejected, but was resolved'), error => {
  assert.sameValue(Object.getPrototypeOf(error), AggregateError.prototype);
  assert(error instanceof AggregateError);
  assert.sameValue(error.errors.length, 0);
}).then($DONE, $DONE);
