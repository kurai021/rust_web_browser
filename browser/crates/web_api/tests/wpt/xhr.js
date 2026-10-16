// Adapted from WPT xhr/open-method-case-sensitive.htm (BSD-3-Clause).
// Only transport scheduling changes: synchronous XHR is explicitly excluded.
['XUNICORN','xUNIcorn','chiCKEN','PATCH','patCH','copy','COpy','inDEX','movE'].forEach(function(method){
  async_test(function(t){var client=new XMLHttpRequest();client.onload=t.step_func_done(function(){assert_equals(client.getResponseHeader('x-request-method'),method);});client.onerror=t.step_func_done(function(){throw new Error('XHR network failure');});client.open(method,'/inspect');client.send(null);},'XMLHttpRequest: open() - case-sensitive methods test ('+method+')');
});
