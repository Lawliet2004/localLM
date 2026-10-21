// Functional browser fixtures only; native persistence is verified separately.
const visualId = 'visual-conversation';
conversations = [{id:visualId,title:'Visual acceptance',updatedAt:Date.now(),providerId:null,modelId:null}];
tasks[visualId] = {projectId:'a',archived:false,pinned:false};
const visualRecord = {messageId:'visual-answer',format:'openui',formatVersion:1,libraryVersion:'locallm.openui/2',sourceText:'LEFT JOIN preserves every left row.',uiProgram:'root = Response([Quiz("joins","SQL joins",[{question:"Keeps unmatched left rows?",options:["Inner","Left"],answer:1,explanation:"LEFT JOIN keeps every left row."}]),Button("Explain further")])',interactiveState:JSON.parse(sessionStorage.getItem('visual-state')||'{}'),generationStatus:'complete'};
messages = [{id:'visual-user',conversationId:visualId,role:'user',content:'Make a joins quiz',reasoning:'',status:'complete',createdAt:1},{id:'visual-answer',conversationId:visualId,role:'assistant',content:visualRecord.sourceText,reasoning:'',status:'complete',createdAt:2,presentation:visualRecord}];
api.getPresentationMode = async () => ({defaultMode:'auto',overrideMode:sessionStorage.getItem('visual-mode'),effectiveMode:sessionStorage.getItem('visual-mode')||'auto'});
api.setPresentationMode = async (id,mode) => { if(mode)sessionStorage.setItem('visual-mode',mode);else sessionStorage.removeItem('visual-mode'); };
api.saveVisualState = async (conversationId,messageId,state) => { if(conversationId!==visualId||messageId!=='visual-answer')throw Error('Ownership mismatch');sessionStorage.setItem('visual-state',JSON.stringify(state)); };
api.rejectPresentation = async () => {};
const visualScenario = sessionStorage.getItem('visual-scenario');
if (visualScenario === 'invalid') visualRecord.uiProgram = 'root = Response([Unknown("bad")])';
if (visualScenario === 'interrupted') {visualRecord.uiProgram = 'root = Response([Text("First"),Chart("Sales","bar",[';visualRecord.generationStatus='interrupted';}
if (visualScenario === 'render-error') {visualRecord.sourceText='Jan sales: 10';visualRecord.uiProgram='root = Response([Chart("Sales","bar",[{label:"Jan",value:10}])])';messages[1].content=visualRecord.sourceText;}
api.rejectPresentation = async (conversationId,messageId,program) => {sessionStorage.setItem('visual-rejection',JSON.stringify({conversationId,messageId,program}));};
