import type { Conversation, Project, WorkspaceIndex } from './types';

/** All folders shown in the sidebar — only projects the user registered. */
export function allProjects(index: WorkspaceIndex): Project[] {
  return index.projects;
}

export function findProject(index: WorkspaceIndex, id: string | null | undefined): Project | undefined {
  return index.projects.find(p => p.id === id);
}

/**
 * The folder a conversation files under: its assigned project when that project
 * still exists, otherwise null — the chat is unfiled.
 */
export function conversationProjectId(index: WorkspaceIndex, conversationId: string): string | null {
  const assigned = index.tasks[conversationId]?.projectId;
  return assigned && findProject(index, assigned) ? assigned : null;
}

/** Group conversations by their resolved folder; unfiled chats sit under `null`. */
export function groupByProject(index: WorkspaceIndex, conversations: Conversation[]): Map<string | null, Conversation[]> {
  const groups = new Map<string | null, Conversation[]>();
  groups.set(null, []);
  for (const project of allProjects(index)) groups.set(project.id, []);
  for (const conversation of conversations) {
    const id = conversationProjectId(index, conversation.id);
    const list = groups.get(id);
    if (list) list.push(conversation);
    else groups.set(id, [conversation]);
  }
  return groups;
}
