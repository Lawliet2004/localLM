import { defaultRuntimeConfig } from '/src/lib/types.ts';
export const nativeAvailable = true;
export const errorMessage = error => error instanceof Error ? error.message : String(error);
const options = window.__auditOptions || {};
let projects = options.noProjects ? [] : [{ id: 'a', name: 'Project A', path: '/fixture/a' }, { id: 'b', name: 'Project B', path: '/fixture/b' }];
let conversations = [], messages = [], tasks = {}, workspace = '', finish;
let waitingPages = [], delayPages = options.delayTrajectoryPage;
const events = Array.from({length: 1200}, (_, i) => ({id:`event-${i+1}`,conversationId:'c1',seq:i+1,runId:null,stepId:null,toolCallId:null,eventType:'fixture',payload:{text:`Event ${i+1}`},ignorable:false,createdAt:i+1}));
const runs = {};
window.__audit = { sends: 0, created: 0, cancelled: 0, workspaces: [], preflights: [] };
window.__audit.releasePages = () => { delayPages = false; waitingPages.splice(0).forEach(resolve => resolve()); };
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
export const api = {
  bootstrap: async () => ({ conversations, config: defaultRuntimeConfig,
    preferences: { runtimePath: '', modelPath: '', projectorPath: '', temperature: 1, topP: 0.95, maxTokens: 2048, systemPrompt: '' },
    runtime: { phase: 'ready', message: 'Loaded', modelPath: '/fixture/model.gguf' }, providers: [], preferredModel: { providerId: null, modelId: '' }, rememberedTools: { sources: [], tools: [], accessMode: 'ask' } }),
  workspaceIndex: async () => ({ projects, tasks }), getWorkspace: async () => ({ path: workspace }),
  setWorkspace: async path => { workspace = path; window.__audit.workspaces.push(path); },
  setTaskWorkspace: async (path, id) => {
    const project = projects.find(p => p.path === path) || { id: 'picked', name: 'Picked folder', path };
    if (!projects.includes(project)) projects.push(project);
    if (id) tasks[id] = { ...tasks[id], projectId: project.id };
    workspace = path; window.__audit.workspaces.push(path);
    return { project, workspace: { projects, tasks } };
  },
  saveTaskMeta: async (id, meta) => { tasks[id] = meta; return { projects, tasks }; },
  createConversation: async () => {
    window.__audit.created++;
    await delay(options.prepareDelay || 0);
    const conversation = { id: `c${window.__audit.created}`, title: 'Audit conversation', updatedAt: Date.now(), providerId: null, modelId: null };
    conversations.unshift(conversation); return conversation;
  },
  messages: async id => messages.filter(m => m.conversationId === id),
  saveConversationTools: async () => {}, saveRememberedTools: async () => {}, conversationTools: async () => ({ sources: [], tools: [], accessMode: 'ask' }),
  sendMessage: async (id, content) => {
    window.__audit.sends++;
    window.__audit.sentWorkspace = projects.find(p => p.id === tasks[id]?.projectId)?.path;
    messages.push({ id: `u${window.__audit.sends}`, conversationId: id, role: 'user', content, reasoning: '', status: 'complete', createdAt: Date.now() });
    const answer = { id: `r${window.__audit.sends}`, conversationId: id, role: 'assistant', content: '', reasoning: '', status: 'error', createdAt: Date.now() };
    messages.push(answer);
    if (options.hold) {
      runs[id] = { id: 'run', conversationId: id, status: 'generating', createdAt: Date.now() };
      await new Promise(resolve => { finish = resolve; });
      answer.status = runs[id].status === 'paused' ? 'paused' : 'interrupted';
      return;
    }
    throw Error('Fixture inference failure after prompt saved');
  },
  cancelGeneration: async () => { window.__audit.cancelled++; finish?.(); },
  pauseGeneration: async () => {
    if (options.pauseFailure) throw Error('Pause unavailable');
    await delay(150);
    for (const run of Object.values(runs)) run.status = 'paused';
    finish?.();
  },
  resumeGeneration: async () => ({ pending: ['Continue'], completed: [], checkpoint: 'Saved' }),
  getConversationRun: async id => runs[id] || null,
  listConnectors: async () => [], listSkills: async () => [], listInstalledModels: async () => [], listCapabilities: async () => [], hasDaytonaKey: async () => false,
  getAutomationConfig: async () => ({ browserEnabled: false, computerEnabled: false }),
  contextPreflight: async (...args) => { window.__audit.preflights.push(args); return { available: false }; },
  getTodos: async () => [], getGoal: async () => null, getSessionEvents: async (id, from, limit) => { if (!options.trajectory) return []; if (delayPages && from === 501) await new Promise(resolve => waitingPages.push(resolve)); return events.filter(event => event.conversationId === id && event.seq >= (from || 0)).slice(0, limit || 1000); }, compactionStatus: async () => ({ auto: false, keepLast: 10, checkpoint: null }),
  modelInstallStatus: async () => ({ busy: false, phase: 'idle' }), runtimeInstallStatus: async () => ({ busy: false, phase: 'idle' }),
};
