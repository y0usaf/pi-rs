// PLAN 9.3: Pi-generated oracle for the complete ExtensionRunner fold contract.
import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { basename, join } from "node:path";
import { DefaultResourceLoader } from "../../ref/pi/packages/coding-agent/src/core/resource-loader.ts";
import { emitProjectTrustEvent } from "../../ref/pi/packages/coding-agent/src/core/extensions/runner.ts";
import { fauxAssistantMessage } from "../../ref/pi/packages/ai/src/providers/faux.ts";
import { createHarness } from "../../ref/pi/packages/coding-agent/test/suite/harness.ts";

const root = mkdtempSync(join(tmpdir(), "pi-rs-extension-events-"));
const first = join(root, "01-first.ts");
const second = join(root, "02-second.ts");
writeFileSync(first, `
export default function (pi: any) {
  (globalThis as any).__eventTrace = [];
  const log = (event: any) => (globalThis as any).__eventTrace.push("first:" + event.type);
  for (const type of ["session_start","session_compact","session_shutdown","session_tree","after_provider_response","agent_start","agent_end","turn_start","turn_end","message_start","message_update","tool_execution_start","tool_execution_update","tool_execution_end","model_select","thinking_level_select"]) pi.on(type as any, async (event: any) => { log(event); if (type === "agent_end") throw new Error("first agent error"); });
  pi.on("resources_discover", async (event: any) => { log(event); return {skillPaths:["skills-a"]}; });
  pi.on("session_before_switch", async (event: any) => { log(event); return {cancel:false,owner:"first"}; });
  pi.on("session_before_fork", async (event: any) => { log(event); return {cancel:false}; });
  pi.on("session_before_compact", async (event: any) => { log(event); return {compaction:{summary:"hook summary",firstKeptEntryId:"keep",tokensBefore:9,details:{owner:"first"}}}; });
  pi.on("session_before_tree", async (event: any) => { log(event); return {customInstructions:"first instructions",label:"first-label"}; });
  pi.on("context", async (event: any) => { log(event); return {messages:[...event.messages,{role:"user",content:"first",timestamp:0}]}; });
  pi.on("before_provider_request", async (event: any) => { log(event); return {...event.payload,first:true}; });
  pi.on("before_agent_start", async (event: any) => { log(event); return {message:{customType:"first",content:"notice",display:true},systemPrompt:event.systemPrompt+"|first"}; });
  pi.on("message_end", async (event: any) => { log(event); if (event.message.role !== "assistant" || event.message.content?.some?.((part:any) => part.type === "toolCall")) return; return {message:{...event.message,content:[{type:"text",text:"first replacement"}]}}; });
  pi.registerTool({name:"event_tool",label:"Event Tool",description:"event seam",parameters:{type:"object",properties:{value:{type:"string"}},required:["value"]},async execute(_id:string,input:any){return {content:[{type:"text",text:"tool:"+input.value}],details:{base:true}};}});\n  pi.on("tool_call", async (event: any) => { log(event); if (event.input.command) event.input.command += " --first"; else event.input.first=true; return {owner:"first"}; });
  pi.on("tool_result", async (event: any) => { log(event); return {content:[{type:"text",text:"first result"}],details:{first:true}}; });
  pi.on("user_bash", async (event: any) => { log(event); return undefined; });
  pi.on("input", async (event: any) => { log(event); return {action:"transform",text:event.text+"|first"}; });
  pi.on("project_trust", async (event: any) => { log(event); return {trusted:"undecided"}; });
  pi.registerCommand("event-trace", {handler: async () => (globalThis as any).__eventTrace});
}`);
writeFileSync(second, `
export default function (pi: any) {
  const log = (event: any) => (globalThis as any).__eventTrace.push("second:" + event.type);
  for (const type of ["session_start","session_compact","session_shutdown","session_tree","after_provider_response","agent_start","agent_end","turn_start","turn_end","message_start","message_update","tool_execution_start","tool_execution_update","tool_execution_end","model_select","thinking_level_select"]) pi.on(type as any, async (event: any) => log(event));
  pi.on("resources_discover", async (event: any) => { log(event); return {promptPaths:["prompts-b"],themePaths:["themes-b"]}; });
  pi.on("session_before_switch", async (event: any) => { log(event); return {cancel:true,owner:"second"}; });
  pi.on("session_before_switch", async () => { (globalThis as any).__eventTrace.push("second:after-cancel"); });
  pi.on("session_before_fork", async (event: any) => { log(event); return {cancel:true}; });
  pi.on("session_before_compact", async (event: any) => { log(event); return {cancel:true}; });
  pi.on("session_before_tree", async (event: any) => { log(event); return {summary:{summary:"tree summary",details:{second:true}},replaceInstructions:true,label:"second-label"}; });
  pi.on("context", async (event: any) => { log(event); return {messages:[...event.messages,{role:"user",content:"second",timestamp:0}]}; });
  pi.on("before_provider_request", async (event: any) => { log(event); return {...event.payload,second:event.payload.first}; });
  pi.on("before_agent_start", async (event: any) => { log(event); return {message:{customType:"second",content:event.systemPrompt,display:false},systemPrompt:event.systemPrompt+"|second"}; });
  pi.on("message_end", async (event: any) => { log(event); return {message:{role:"user",content:"invalid",timestamp:0}}; });
  pi.on("tool_call", async (event: any) => { log(event); return event.toolName === "bash" ? {block:true,reason:event.input.command} : undefined; });
  pi.on("tool_result", async (event: any) => { log(event); return {isError:true,details:{second:event.details.first}}; });
  pi.on("user_bash", async (event: any) => { log(event); return {result:{output:"handled bash",exitCode:7,cancelled:false,truncated:false}}; });
  pi.on("input", async (event: any) => { log(event); return event.text.includes("handle") ? {action:"handled"} : {action:"transform",text:event.text+"|second",images:event.images}; });
  pi.on("project_trust", async (event: any) => { log(event); return {trusted:"yes",remember:true}; });
}`);

const stable = (path: string) => basename(path).replace(/\.[^.]+$/, "");
const clean = (value: any): any => {
  if (Array.isArray(value)) return value.map(clean);
  if (value && typeof value === "object") return Object.fromEntries(Object.entries(value).map(([k,v]) => [k, clean(v)]));
  if (typeof value === "string") return value.replaceAll(first, stable(first)).replaceAll(second, stable(second));
  return value;
};

async function main() {
  const loader = new DefaultResourceLoader({cwd:root, agentDir:root, additionalExtensionPaths:[first,second], noSkills:true, noPromptTemplates:true, noThemes:true, noContextFiles:true});
  await loader.reload();
  const loaded = loader.getExtensions();
  const harness = await createHarness({resourceLoader:loader});
  try {
    const runner = harness.session.extensionRunner;
    const errors:any[] = [];
    runner.onError((error:any) => errors.push({extensionPath:stable(error.extensionPath),event:error.event,error:error.error}));
    const genericTypes = ["session_start","session_compact","session_shutdown","session_tree","after_provider_response","agent_start","agent_end","turn_start","turn_end","message_start","message_update","tool_execution_start","tool_execution_update","tool_execution_end","model_select","thinking_level_select"];
    for (const type of genericTypes) await runner.emit({type,status:201,headers:{x:"y"},messages:[],turnIndex:2,timestamp:123,message:{role:"assistant",content:[],timestamp:0},toolResults:[],toolCallId:"call",toolName:"bash",args:{command:"x"},partialResult:{content:[]},result:{content:[]},isError:false,model:harness.model,previousModel:undefined,source:"set",level:"low",previousLevel:"off",newLeafId:"leaf",oldLeafId:"old",fromExtension:false,compactionEntry:{id:"compact"}} as any);
    const beforeSwitch = await runner.emit({type:"session_before_switch",reason:"resume",targetSessionFile:"target.jsonl"});
    const beforeFork = await runner.emit({type:"session_before_fork",entryId:"entry",position:"before"});
    const beforeCompact = await runner.emit({type:"session_before_compact",preparation:{firstKeptEntryId:"keep"},branchEntries:[],signal:new AbortController().signal} as any);
    const beforeTree = await runner.emit({type:"session_before_tree",preparation:{targetId:"target"},signal:new AbortController().signal} as any);
    const context = await runner.emitContext([{role:"user",content:"base",timestamp:0}] as any);
    const payload = await runner.emitBeforeProviderRequest({base:true});
    const beforeAgent = await runner.emitBeforeAgentStart("prompt",undefined,"system",{cwd:root});
    const message = await runner.emitMessageEnd({type:"message_end",message:{role:"assistant",content:[{type:"text",text:"base"}],api:"x",provider:"x",model:"x",usage:{},stopReason:"stop",timestamp:0}} as any);
    const toolInput:any = {command:"echo"};
    const toolCall = await runner.emitToolCall({type:"tool_call",toolCallId:"call",toolName:"bash",input:toolInput});
    const toolResult = await runner.emitToolResult({type:"tool_result",toolCallId:"call",toolName:"bash",input:toolInput,content:[{type:"text",text:"base result"}],details:{base:true},isError:false} as any);
    const userBash = await runner.emitUserBash({type:"user_bash",command:"echo hi",excludeFromContext:false,cwd:root});
    const input = await runner.emitInput("go",undefined,"interactive");
    const handledInput = await runner.emitInput("handle",undefined,"interactive","steer");
    const trust = await emitProjectTrustEvent(loaded,{type:"project_trust",cwd:root},{cwd:root,mode:"tui",hasUI:false,ui:runner.getUIContext()});
    const resources = await runner.emitResourcesDiscover(root,"startup");
    const trace = [...await runner.getCommand("event-trace")!.handler("",runner.createCommandContext())];
    const traceBeforeProduct = trace.length;
    const foldErrorCount = errors.length;
    harness.setResponses([
      fauxAssistantMessage({type:"toolCall",id:"event-call",name:"event_tool",arguments:{value:"x"}}, {timestamp:0}),
      fauxAssistantMessage("done", {timestamp:0}),
    ]);
    await harness.session.prompt("go");
    const fullTrace = await runner.getCommand("event-trace")!.handler("",runner.createCommandContext());
    const significant = new Set(["session_start","resources_discover","input","before_agent_start","agent_start","turn_start","message_start","message_end","after_provider_response","tool_execution_start","tool_call","tool_result","tool_execution_end","turn_end","agent_end"]);
    const productTrace = fullTrace.slice(traceBeforeProduct).filter((entry:string) => significant.has(entry.slice(entry.indexOf(":")+1)));
    process.stdout.write(JSON.stringify(clean({beforeSwitch,beforeFork,beforeCompact,beforeTree,context,payload,beforeAgent,message,toolInput,toolCall,toolResult,userBash,input,handledInput,trust:trust.result,resources,errors,foldErrorCount,trace,productTrace}),null,"\t")+"\n");
  } finally { harness.cleanup(); }
}
main().catch((error) => { console.error(error); process.exitCode = 1; });
