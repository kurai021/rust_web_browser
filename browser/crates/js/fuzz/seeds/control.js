var n=0;try{for(let i=0;i<5;i++){n+=i;}throw new Error('caught');}catch(e){n++;}finally{n++;}n;
