// Copyright (C) 2018 André Bargull. All rights reserved.
// BSD-3-Clause; ../LICENSE. Upstream: language/expressions/import.meta/distinct-for-each-module.js
import {meta as fixture_meta, getMeta} from "./distinct-for-each-module_FIXTURE.js";
assert.notSameValue(import.meta, fixture_meta, "foreign import.meta accessed via import binding");
assert.notSameValue(import.meta, getMeta(), "foreign import.meta accessed via function call");
assert.sameValue(fixture_meta, getMeta(), "import.meta accessed via import binding is identical to the one accessed via call");
