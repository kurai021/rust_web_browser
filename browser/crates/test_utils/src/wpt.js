// Restricted testharness adapter; the page APIs and interpreter remain real.
var wpt_cases=[], wpt_current=null;
function wpt_case(name){var item={name:name,status:'pending',assertions:0,detail:null};wpt_cases.push(item);return item;}
function wpt_fail(item,error){item.status='fail';item.detail=String(error);}
function test(fn,name){var item=wpt_case(name);var previous=wpt_current;wpt_current=item;try{fn();item.status='pass';}catch(e){wpt_fail(item,e);}wpt_current=previous;}
function skip(name,reason){var item=wpt_case(name);item.status='skip';item.detail=reason;}
function async_test(fn,name){var item=wpt_case(name);var t={done:function(){if(item.status==='pending')item.status='pass';},step_func:function(fn){return function(){var previous=wpt_current;wpt_current=item;var result;try{result=fn.apply(this,arguments);}catch(e){wpt_fail(item,e);}wpt_current=previous;return result;};},step_func_done:function(fn){return t.step_func(function(){fn.apply(this,arguments);t.done();});}};t.step_func(fn)(t);return t;}
function promise_test(fn,name){async_test(function(t){var result=fn(t);result.then(t.step_func_done(function(){}),t.step_func_done(function(e){throw e;}));},name);}
function assert_equals(a,b,message){if(wpt_current)wpt_current.assertions++;if(a!==b)throw new Error(message||('Expected '+b+', got '+a));}
function assert_true(value,message){assert_equals(value,true,message);}
function assert_false(value,message){assert_equals(value,false,message);}
function assert_array_equals(a,b,message){assert_equals(JSON.stringify(a),JSON.stringify(b),message);}
function assert_throws_js(type,fn){var name='';try{fn();}catch(e){name=e.name;}assert_equals(name,type.name);}
function assert_throws_dom(name,fn){var actual='';try{fn();}catch(e){actual=e.name;}assert_equals(actual,name);}
