import assert from "node:assert/strict";
import test from "node:test";
import {ExternalToolExecutor, InMemoryTransport, RoderRpcClient} from "../src/index.js";
import type {JsonRpcNotification, ToolExecutorLease} from "../src/index.js";

const lease: ToolExecutorLease = {threadId:"thread",leaseId:"lease",generation:1,contractVersion:1};
function fixture() {
  const resolutions: unknown[] = [];
  const client = new RoderRpcClient(new InMemoryTransport(request => {
    if (request.method === "tools/resolve") resolutions.push(request.params);
    return {jsonrpc:"2.0",id:request.id,result:request.method === "tools/bind_executor" ? {executor:lease} : {resolved:true}};
  }));
  return {client,resolutions};
}
function request(executor: ToolExecutorLease = lease): JsonRpcNotification {
  return {jsonrpc:"2.0",method:"thread/toolExecutionRequested",params:{executor,
    threadId:"thread",turnId:"turn",requestId:"request",call:{id:"call",name:"edit",arguments:{value:1}}}};
}

test("executes once and resolves with exact lease and turn", async () => {
  const {client,resolutions} = fixture();
  let calls = 0;
  const executor = await ExternalToolExecutor.bind(client,"thread",() => {calls++;return {output:"saved"};});
  await Promise.all([executor.handle(request()),executor.handle(request())]);
  assert.equal(calls,1);
  assert.deepEqual(resolutions,[{executor:lease,turnId:"turn",requestId:"request",output:"saved",isError:false}]);
});

test("wrong lease never invokes callback", async () => {
  const {client,resolutions} = fixture();
  const executor = await ExternalToolExecutor.bind(client,"thread",() => {throw new Error("must not execute");});
  await executor.handle(request({...lease,generation:2}));
  assert.equal(resolutions.length,0);
});

for (const kind of ["disconnect","revoke","timeout"]) {
  test(`${kind} aborts pending callback and prevents late resolution`, async () => {
    const {client,resolutions} = fixture();
    let signal: AbortSignal | undefined;
    let finish!: () => void;
    const work = new Promise<void>(resolve => {finish=resolve;});
    const executor = await ExternalToolExecutor.bind(client,"thread",async (_,context) => {
      signal=context.signal;await work;return {output:"late"};
    });
    const running = executor.handle(request());
    assert.equal(signal?.aborted,false);
    if (kind === "disconnect") executor.stop();
    else await executor.handle({jsonrpc:"2.0",method:kind === "revoke" ? "tools/executorRevoked" : "thread/toolExecutionResolved",
      params:{executor:lease,threadId:"thread",requestId:"request",outcome:"timedOut"}});
    assert.equal(signal?.aborted,true);
    finish();await running;
    await executor.handle(request());
    assert.equal(resolutions.length,0);
  });
}

test("hosted RoderAgent binds before turn start and uses the bound helper", async () => {
  const {RoderAgent} = await import("../src/index.js");
  const methods: string[] = [];
  const transport = new InMemoryTransport(request => {
    methods.push(request.method);
    const result = request.method === "thread/start" ? {thread:{id:"thread"}} :
      request.method === "tools/bind_executor" ? {executor:lease} :
      request.method === "turn/start" ? {turn:{id:"turn"}} : {resolved:true};
    return {jsonrpc:"2.0",id:request.id,result};
  });
  const agent = await RoderAgent.create({transport,workspaceId:"workspace",externalToolExecution:"hosted",
    externalTools:[{name:"edit",description:"edit",parameters:{type:"object"}}],onToolExecute:(_,context) => {
      assert.equal(context?.threadId,"thread");return {output:"done"};
    }});
  await agent.send("edit");
  assert.deepEqual(methods,["thread/start","tools/bind_executor","turn/start"]);
  transport.emit(request());
  for (let i=0;i<20 && !methods.includes("tools/resolve");i++) await new Promise(resolve=>setTimeout(resolve,1));
  assert.ok(methods.includes("tools/resolve"));
  await agent.close();
  assert.equal(methods.at(-1),"tools/unbind_executor");
});

test("a late completion of another turn cannot abort the active call", async () => {
  const {client,resolutions} = fixture();
  let signal: AbortSignal | undefined;
  let finish!: () => void;
  const waiting = new Promise<void>(resolve => {finish=resolve;});
  const executor = await ExternalToolExecutor.bind(client,"thread",async (_,context)=> {
    signal=context.signal;await waiting;return {output:"done"};
  });
  const running = executor.handle(request());
  await executor.handle({jsonrpc:"2.0",method:"turn/completed",params:{threadId:"thread",turn:{id:"old-turn"}}});
  assert.equal(signal?.aborted,false);
  finish();await running;
  assert.equal(resolutions.length,1);
});

test('concurrent initial sends share one thread and hosted binding', async () => {
  const {RoderAgent} = await import('../src/index.js');
  const methods: string[] = [];
  const transport = new InMemoryTransport(async request => {
    methods.push(request.method);
    await new Promise(resolve=>setTimeout(resolve,1));
    const result=request.method === 'thread/start' ? {thread:{id:'thread'}} :
      request.method === 'tools/bind_executor' ? {executor:lease} : {turn:{id:'turn'}};
    return {jsonrpc:'2.0',id:request.id,result};
  });
  const agent = await RoderAgent.create({transport,workspaceId:'workspace',externalToolExecution:'hosted',onToolExecute:()=>({output:'ok'})});
  await Promise.all([agent.send('one'),agent.send('two')]);
  assert.equal(methods.filter(method=>method==='thread/start').length,1);
  assert.equal(methods.filter(method=>method==='tools/bind_executor').length,1);
  await agent.close();
});

test('a revoked executor permits a fresh bind on the next send and teardown is best effort', async () => {
  const {RoderAgent} = await import('../src/index.js');
  let binds=0;
  const transport = new InMemoryTransport(request => {
    if (request.method==='tools/bind_executor') binds++;
    if (request.method==='tools/unbind_executor') return {jsonrpc:'2.0',id:request.id,error:{code:-32012,message:'executor_not_owned'}};
    const result = request.method==='tools/bind_executor' ? {executor:lease} : {turn:{id:'turn'}};
    return {jsonrpc:'2.0',id:request.id,result};
  });
  const agent=await RoderAgent.create({transport,threadId:'thread',externalToolExecution:'hosted',onToolExecute:()=>({output:'ok'})});
  await agent.send('one');
  transport.emit({jsonrpc:'2.0',method:'tools/executorRevoked',params:{executor:lease}});
  await new Promise(resolve=>setTimeout(resolve,1));
  await agent.send('two');
  assert.equal(binds,2);
  await agent.close();
});

test('transport closure immediately aborts callbacks and suppresses buffered requests', async () => {
  const {client,resolutions} = fixture();
  let signal: AbortSignal | undefined;
  let finish!: () => void;
  let calls = 0;
  const waiting = new Promise<void>(resolve=>{finish=resolve;});
  const executor = await ExternalToolExecutor.bind(client,'thread',async (_,context)=>{
    calls++;signal=context.signal;await waiting;return {output:'late'};
  });
  const running=executor.handle(request());
  client.close();
  assert.equal(signal?.aborted,true);
  assert.equal(executor.isActive,false);
  const buffered=request();
  (buffered.params as Record<string,unknown>).requestId='buffered';
  await executor.handle(buffered);
  assert.equal(calls,1);
  finish();await running;
  assert.equal(resolutions.length,0);
  await assert.rejects(ExternalToolExecutor.bind(client,'thread',()=>({output:'never'})),/closed transport/);
});
