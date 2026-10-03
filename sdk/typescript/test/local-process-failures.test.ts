import assert from 'node:assert/strict';
import test from 'node:test';
import {LocalProcessTransport} from '../src/index.js';

for (const line of ['null', 'invalid-json']) {
  test(`malformed child output ${line} rejects and closes without crashing`, async () => {
    const transport = new LocalProcessTransport({command:process.execPath,args:['-e',
      `process.stdin.once('data',()=>console.log(${JSON.stringify(line)}))`]});
    await assert.rejects(transport.request({jsonrpc:'2.0',id:1,method:'initialize'}),/Invalid app-server/);
    await assert.rejects(transport.request({jsonrpc:'2.0',id:2,method:'initialize'}),/closed/);
    await transport.close();
  });
}

test('a child exit closes both current and future requests', async () => {
  const transport = new LocalProcessTransport({command:process.execPath,args:['-e',"process.stdin.once('data',()=>process.exit(1))"]});
  await assert.rejects(transport.request({jsonrpc:'2.0',id:1,method:'initialize'}),/exited/);
  await assert.rejects(transport.request({jsonrpc:'2.0',id:2,method:'initialize'}),/closed/);
  await transport.close();
});

test('numeric and string request ids have independent pending responses', async () => {
  const script = `require('node:readline').createInterface({input:process.stdin}).on('line',line=> {
    const request=JSON.parse(line);setTimeout(()=>console.log(JSON.stringify({jsonrpc:'2.0',id:request.id,result:{id:request.id}})),10);
  });`;
  const transport = new LocalProcessTransport({command:process.execPath,args:['-e',script]});
  const [number,string] = await Promise.all([
    transport.request({jsonrpc:'2.0',id:1,method:'initialize'}),
    transport.request({jsonrpc:'2.0',id:'1',method:'initialize'}),
  ]);
  assert.deepEqual(number.result,{id:1});assert.deepEqual(string.result,{id:'1'});
  await transport.close();
});

test('serialization failure does not poison a reusable request id', async () => {
  const script = `require('node:readline').createInterface({input:process.stdin}).on('line',line=>{const request=JSON.parse(line);console.log(JSON.stringify({jsonrpc:'2.0',id:request.id,result:{ok:true}}))});`;
  const transport = new LocalProcessTransport({command:process.execPath,args:['-e',script]});
  await assert.rejects(transport.request({jsonrpc:'2.0',id:1,method:'initialize',params:{unserializable:1n}}),/BigInt/);
  const response=await transport.request({jsonrpc:'2.0',id:1,method:'initialize'});
  assert.deepEqual(response.result,{ok:true});
  await transport.close();
});
