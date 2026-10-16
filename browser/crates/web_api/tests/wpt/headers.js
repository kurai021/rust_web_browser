// Adapted from WPT fetch/api/headers/headers-basic.any.js (BSD-3-Clause).
// See PROVENANCE.md for the pinned source and explicit iterator exclusions.
"use strict";
test(function(){new Headers();},"Create headers from no parameter");
test(function(){new Headers(undefined);},"Create headers from undefined parameter");
test(function(){new Headers({});},"Create headers from empty object");
[null,1].forEach(function(parameter){test(function(){assert_throws_js(TypeError,function(){new Headers(parameter);});},"Create headers with "+parameter+" should throw");});
var headerDict={name1:'value1',name2:'value2',name3:'value3',name4:null,name5:undefined,name6:1,'Content-Type':'value4'};
var headerSeq=[];
for(var name in headerDict)headerSeq.push([name,headerDict[name]]);
test(function(){var headers=new Headers(headerSeq);for(name in headerDict)assert_equals(headers.get(name),String(headerDict[name]));assert_equals(headers.get('length'),null);},"Create headers with sequence");
test(function(){var headers=new Headers(headerDict);for(name in headerDict)assert_equals(headers.get(name),String(headerDict[name]));},"Create headers with record");
test(function(){var headers=new Headers(headerDict);var headers2=new Headers(headers);for(name in headerDict)assert_equals(headers2.get(name),String(headerDict[name]));},"Create headers with existing headers");
test(function(){var headers=new Headers();for(name in headerDict){headers.append(name,headerDict[name]);assert_equals(headers.get(name),String(headerDict[name]));}},"Check append method");
test(function(){var headers=new Headers();for(name in headerDict){headers.set(name,headerDict[name]);assert_equals(headers.get(name),String(headerDict[name]));}},"Check set method");
test(function(){var headers=new Headers(headerDict);for(name in headerDict)assert_true(headers.has(name));assert_false(headers.has('nameNotInHeaders'));},"Check has method");
test(function(){var headers=new Headers(headerDict);for(name in headerDict){assert_true(headers.has(name));headers.delete(name);assert_false(headers.has(name));}},"Check delete method");
test(function(){var headers=new Headers(headerDict);for(name in headerDict)assert_equals(headers.get(name),String(headerDict[name]));assert_equals(headers.get('nameNotInHeaders'),null);},"Check get method");
['Create headers with existing headers with custom iterator','Check keys method','Check values method','Check entries method','Check Symbol.iterator method','Check forEach method','Iteration skips elements removed while iterating','Removing elements already iterated over causes an element to be skipped during iteration','Appending a value pair during iteration causes it to be reached during iteration','Prepending a value pair before the current element position causes it to be skipped during iteration and adds the current element a second time'].forEach(function(name){skip(name,'Full iterable/generator semantics are beyond the Phase 7 Level 1 boundary; entries/keys/values are snapshots.');});
