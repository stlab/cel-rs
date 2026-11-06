import assert from "node:assert/strict";
import test from "node:test";
import {createNumericSource, runNumericEvents} from "../src/components/numeric_events.js";

/** Provides one local text input with synchronous native snapshot fields. */
class Field extends EventTarget {
    value = "12";
    selectionStart = 0;
    selectionEnd = 2;
    selectionDirection = "none";
    disabled = false;
    readOnly = false;
    isConnected = true;
    /** Applies a native text selection without changing the value. */
    setSelectionRange(start,end,direction) {
        this.selectionStart=start;this.selectionEnd=end;this.selectionDirection=direction;
    }
    /** Dispatches a source event after applying the supplied native state. */
    fire(type, fields = {}, value = this.value, start = value.length, end = start) {
        this.value = value;
        this.selectionStart = start;
        this.selectionEnd = end;
        const event = new Event(type);
        for (const [key, data] of Object.entries(fields)) Object.defineProperty(event, key, {value:data});
        this.dispatchEvent(event);
    }
}

/** Installs the actual source adapter and retains its real outgoing frames. */
function mount(value = "12", start = 0, end = value.length) {
    const field = new Field();
    field.value = value; field.selectionStart = start; field.selectionEnd = end;
    const frames = [];
    const source = createNumericSource(field, "18446744073709551615", frame => frames.push(frame));
    const writes = () => frames.filter(f => f.action === "input" || f.action === "end").map(f=>f.value);
    return {field, frames, source, writes};
}

test("delayed post-end final never submits captured numeric intermediate", async () => {
    const {field, source, writes} = mount();
    field.fire("compositionstart", {}, "12", 0, 2);
    field.fire("input", {isComposing:true,inputType:"insertCompositionText",data:"17"}, "17");
    field.fire("compositionend", {data:"19"});
    await new Promise(resolve=>setTimeout(resolve,30));
    assert.deepEqual(writes(), []);
    field.fire("input", {isComposing:false,inputType:"insertCompositionText",data:"19"}, "19");
    assert.deepEqual(writes(), ["19"]);
    source.close();
});

test("pre-end full value commits once and terminal echo does not eat equal ordinary edits", () => {
    const {field, source, writes} = mount("121212",2,4);
    field.fire("compositionstart", {}, "121212",2,4);
    field.fire("input", {isComposing:true,inputType:"insertCompositionText",data:"19"}, "121912",4);
    field.fire("compositionend", {data:"19"});
    assert.deepEqual(writes(), ["121912"]);
    field.fire("input", {isComposing:false,inputType:"insertCompositionText",data:"19"});
    assert.deepEqual(writes(), ["121912"]);
    field.fire("beforeinput", {isComposing:false,inputType:"insertText",data:"2"});
    field.fire("input", {isComposing:false,inputType:"insertText",data:"2"});
    assert.deepEqual(writes(), ["121912","121912"]);
    source.close();
});

test("collapsed insertion and authoritative beforeinput range expansion retain actual snapshots", () => {
    const {field, source, writes} = mount("1234",2,2);
    field.fire("compositionstart",{},"1234",2,2);
    field.fire("beforeinput",{isComposing:true,inputType:"insertCompositionText",data:"99"},"1234",1,3);
    field.fire("input",{isComposing:true,inputType:"insertCompositionText",data:"99"},"1994");
    field.fire("compositionend",{data:"99"});
    assert.deepEqual(writes(),["1994"]);
    source.close();
});

test("outside-span edits report a limitation without guessing a full value", () => {
    const {field, frames, source, writes} = mount("121212",2,4);
    field.fire("compositionstart",{},"121212",2,4);
    field.fire("input",{isComposing:true,inputType:"insertCompositionText",data:"19"},"191212");
    field.fire("compositionend",{data:"19"});
    assert.deepEqual(writes(),[]);
    assert(frames.some(f=>f.action==="error" && f.message.includes("range")));
    source.close();
});

test("empty completion deletes but cancelled unchanged composition does not clear", () => {
    for (const [final, expected] of [["",[""]],["12",[]]]) {
        const {field, source, writes} = mount();
        field.fire("compositionstart",{},"12",0,2);
        field.fire("input",{isComposing:true,inputType:"deleteCompositionText",data:""},final);
        field.fire("compositionend",{data:""});
        assert.deepEqual(writes(),expected);
        source.close();
    }
});

test("cancelled collapsed insertion is not an equal-valued aspiration", () => {
    const {field,source,writes}=mount("12",1,1);
    field.fire("compositionstart",{},"12",1,1);
    field.fire("compositionend",{data:""},"12",1,1);
    assert.deepEqual(writes(),[]);
    source.close();
});

test("empty end on unchanged text waits for an authoritative delayed deletion", async () => {
    const {field,source,writes}=mount();
    field.fire("compositionstart",{},"12",0,2);
    field.fire("compositionend",{data:""});
    await new Promise(resolve=>setTimeout(resolve,30));
    assert.deepEqual(writes(),[]);
    field.fire("input",{inputType:"deleteCompositionText",data:"",isComposing:false},"");
    assert.deepEqual(writes(),[""]);
    source.close();
});

test("reset retires queued session, acknowledges epoch and permits next ordinary edit", () => {
    const {field, frames, source, writes} = mount();
    field.fire("compositionstart",{},"12",0,2);
    field.fire("input",{isComposing:true,inputType:"insertCompositionText",data:"17"},"17");
    field.fire("compositionend",{data:"19"});
    source.receive({kind:"reset",epoch:"2",value:"99",disabled:true,readonly:false});
    assert(frames.some(f=>f.action==="ready"&&f.epoch==="2"));
    field.fire("input",{isComposing:false,inputType:"insertCompositionText",data:"19"},"19");
    source.receive({kind:"reset",epoch:"3",value:"99",disabled:false,readonly:false});
    field.fire("input",{isComposing:false,inputType:"insertCompositionText",data:"19"},"19");
    assert.deepEqual(writes(),[]);
    assert.equal(field.value,"99","retired terminal cannot replace authoritative reset display");
    field.fire("beforeinput",{isComposing:false,inputType:"insertText",data:"7"});
    field.fire("input",{isComposing:false,inputType:"insertText",data:"7"},"7");
    assert.deepEqual(writes(),["7"]);
    assert.equal(frames.at(-1).epoch,"3");
    source.close();
});

test("retired terminal cannot overwrite a newer ordinary incomplete draft", () => {
    const {field,source,writes}=mount();
    field.fire("beforeinput",{inputType:"insertText",isComposing:false,data:"-"});
    field.fire("input",{inputType:"insertText",isComposing:false,data:"-"},"-");
    field.fire("input",{inputType:"insertCompositionText",isComposing:false,data:"19"},"19");
    assert.equal(field.value,"-");
    assert.deepEqual(writes(),["-"]);
    source.close();
});

test("blur, new ordinary input and teardown cancel a pending completion", () => {
    for (const cancel of ["blur","ordinary","close"]) {
        const {field, source, writes} = mount();
        field.fire("compositionstart",{},"12",0,2);
        field.fire("input",{isComposing:true,inputType:"insertCompositionText",data:"17"},"17");
        field.fire("compositionend",{data:"19"});
        if(cancel==="close") source.close();
        else if(cancel==="blur") field.fire("blur");
        else {
            field.fire("beforeinput",{isComposing:false,inputType:"insertText",data:"5"});
            field.fire("input",{isComposing:false,inputType:"insertText",data:"5"},"5");
        }
        field.fire("input",{isComposing:false,inputType:"insertCompositionText",data:"19"},"19");
        assert.deepEqual(writes(),cancel==="ordinary"?["5"]:[]);
        source.close();
    }
});

test("display is sequence-guarded and never overwrites an active composing draft", () => {
    const {field, frames, source} = mount();
    source.receive({kind:"display",epoch:"0",sequence:frames.at(-1).sequence,value:"99"});
    assert.equal(field.value,"99");
    field.fire("compositionstart",{},"99",0,2);
    field.fire("input",{isComposing:true,inputType:"insertCompositionText",data:"17"},"17");
    source.receive({kind:"display",epoch:"0",sequence:"1",value:"5"});
    assert.equal(field.value,"17");
    source.close();
    field.fire("input",{inputType:"insertText",isComposing:false},"8");
    assert.equal(frames.at(-1).action,"draft");
});

test("setup failure reports explicitly and failed event send closes its owned listener scope", async () => {
    const messages=[];
    await runNumericEvents({document:{querySelector:()=>null}},"9",{send:m=>messages.push(m)});
    assert.equal(messages[0].action,"fatal");
    assert.match(messages[0].message,/not mounted/);
    let sends=0;
    const field=new Field();
    const source=createNumericSource(field,"9",()=>{
        if(++sends>1)throw new Error("transport lost");
    });
    field.fire("input",{inputType:"insertText",isComposing:false},"19");
    await source.closed;
    const before=sends;
    field.fire("input",{inputType:"insertText",isComposing:false},"20");
    assert.equal(sends,before,"all owned listeners were removed");
});

test("async host commands never read the native value or overwrite newer source text", () => {
    const {field,frames,source}=mount();
    Object.defineProperty(field,"value",{get(){throw new Error("asynchronous DOM-value read");},set(){}});
    assert.doesNotThrow(()=>source.receive({kind:"display",epoch:"0",sequence:frames.at(-1).sequence,value:"99"}));
    assert.doesNotThrow(()=>source.receive({kind:"reset",epoch:"1",value:"7",disabled:false,readonly:false}));
    assert.equal(frames.at(-1).value,"7");
    source.close();
});

test("receive failure removes the actual listener owner and reports a terminal source error", async () => {
    const field=new Field(), frames=[];
    await runNumericEvents({document:{querySelector:()=>field}},"9",{
        send:frame=>frames.push(frame),
        recv:async()=>{throw new Error("desktop channel unavailable");},
    });
    assert.equal(frames.at(-1).action,"fatal");
    assert.match(frames.at(-1).message,/desktop channel unavailable/);
    const before=frames.length;
    field.fire("input",{inputType:"insertText",isComposing:false},"19");
    assert.equal(frames.length,before);
});
