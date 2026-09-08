import { readFileSync, writeFileSync } from 'node:fs';
import { parse } from 'yaml';

const connectors = parse(readFileSync('catalog/mcp-catalog.yaml', 'utf8')).mcp_servers;
const skills = parse(readFileSync('catalog/skill-catalog.yaml', 'utf8')).skills;
writeFileSync('src/lib/catalog.json', JSON.stringify({ connectors, skills }, null, 2) + '\n');
