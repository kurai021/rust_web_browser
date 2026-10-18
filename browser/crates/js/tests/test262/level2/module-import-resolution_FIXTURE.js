// Copyright (C) 2019 Leo Balter. All rights reserved.
// BSD-3-Clause; ../LICENSE. Upstream: language/module-code/top-level-await/module-import-resolution_FIXTURE.js
await 1;
await 2;
export default await Promise.resolve(42);
export const y = await 39;
export const x = await 'named';
if (false) { await Promise.reject(42); }
