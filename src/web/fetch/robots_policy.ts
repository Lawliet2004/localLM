export interface RobotsRule { userAgent: string; disallow: string[]; allow: string[] }
/** RFC 9309 group selection and longest matching path, including * and $. */
export class RobotsChecker {
  parseRobotsTxt(text: string): RobotsRule[] {
    const rules: RobotsRule[] = [];
    let agents: string[] = [], allow: string[] = [], disallow: string[] = [], directives = false;
    const flush = () => { for (const userAgent of agents) rules.push({userAgent, allow:[...allow], disallow:[...disallow]}); agents=[]; allow=[]; disallow=[]; directives=false; };
    for (const raw of text.split(/\r?\n/)) {
      const line = raw.split('#')[0].trim();
      const colon = line.indexOf(':');
      if (colon < 0) continue;
      const key = line.slice(0,colon).trim().toLowerCase(), value = line.slice(colon+1).trim();
      if (key === 'user-agent') { if (directives) flush(); agents.push(value.toLowerCase()); }
      else if (agents.length && (key === 'allow' || key === 'disallow')) { directives=true; if(value) (key === 'allow' ? allow : disallow).push(value); }
    }
    flush(); return rules;
  }
  isPathAllowed(path: string, rules: RobotsRule[], userAgent = '*'): boolean {
    const explicit = rules.filter(r => r.userAgent !== '*' && userAgent.toLowerCase().includes(r.userAgent));
    const longest = Math.max(0, ...explicit.map(r => r.userAgent.length));
    const selected = explicit.length ? explicit.filter(r => r.userAgent.length === longest) : rules.filter(r => r.userAgent === '*');
    let bestLength = -1, allowed = true;
    for (const rule of selected) for (const [patterns, permit] of [[rule.disallow,false],[rule.allow,true]] as const) {
      for (const pattern of patterns) {
        const end = pattern.endsWith('$');
        const body = end ? pattern.slice(0,-1) : pattern;
        const regex = '^' + body.split('*').map(p => p.replace(/[.*+?^${}()|[\]\\]/g,'\\$&')).join('.*') + (end ? '$' : '');
        if (new RegExp(regex).test(path) && (body.length > bestLength || (body.length === bestLength && permit))) { bestLength=body.length; allowed=permit; }
      }
    }
    return allowed;
  }
}
