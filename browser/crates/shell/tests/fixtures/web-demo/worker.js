onmessage=function(event){var total=event.data.reduce(function(sum,row){return sum+row.value;},0);postMessage({total:total,dom:typeof document});};
