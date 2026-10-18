// Copyright (C) 2019 Leo Balter. All rights reserved.
// BSD-3-Clause; ../LICENSE. Upstream: language/module-code/top-level-await/module-import-resolution.js
import foo from './module-import-resolution_FIXTURE.js';
assert.sameValue(foo, 42);
import {x, y} from './module-import-resolution_FIXTURE.js';
assert.sameValue(x, 'named');
assert.sameValue(y, 39);
$DONE();
