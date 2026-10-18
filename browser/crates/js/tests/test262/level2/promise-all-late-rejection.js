// Copyright (C) 2019 Leo Balter, 2020 Rick Waldron. All rights reserved.
// BSD-3-Clause; ../LICENSE. Upstream: built-ins/Promise/all/resolve-ignores-late-rejection.js
var resolver = { then(resolve) { resolve(42); } };
var lateRejector = { then(resolve, reject) { resolve(33); reject(); } };
Promise.all([resolver, lateRejector]).then(resolution => {
  assert.sameValue(resolution[0], 42);
  assert.sameValue(resolution[1], 33);
}).then($DONE, $DONE);
