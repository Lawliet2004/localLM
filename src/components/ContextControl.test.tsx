import { render, screen } from '@testing-library/react';
import { expect, it } from 'vitest';
import { ContextControl } from './ContextControl';

const fullUsage = {
  inputTokens: 19469,
  responseReserve: 262144,
  contextLength: 1048576,
  outputTokens: 1204,
  breakdown: { messages: 2100, systemTools: 7000, mcpTools: 1200, systemPrompt: 4800, skills: 2100, metaContext: 269 },
  avgCacheHitRate: 0.93,
};

it('counts only tokens actually in the window, never the response reserve', () => {
  // A fresh conversation once displayed 27% here because the 256K response
  // reserve was added to the ring; the window holds ~20K real tokens.
  render(<ContextControl usage={fullUsage} />);
  expect(screen.getByRole('button', { name: 'Context 2% used — hover or click for details' }))
    .toBeInTheDocument();
  expect(screen.getByText('20.7K/1M (2%)')).toBeInTheDocument();
});

it('breaks the window down by category with shares and cache metrics', () => {
  render(<ContextControl usage={fullUsage} />);
  expect(screen.getByText('Context windows')).toBeInTheDocument();
  const rows = ['Messages', 'System tools', 'MCP tools', 'System prompt', 'Skills', 'Meta context'];
  const expected = ['12%', '40.1%', '6.9%', '27.5%', '12%', '1.5%'];
  rows.forEach((label, index) => {
    expect(screen.getByText(label).parentElement).toHaveTextContent(expected[index]);
  });
  const footer = screen.getByText('Average cache hit rate').parentElement;
  expect(footer).toHaveTextContent('93%');
  expect(screen.getByText('Input').parentElement).toHaveTextContent('19,469');
  expect(screen.getByText('Output').parentElement).toHaveTextContent('1,204');
});

it('marks estimated totals and falls back to the live round cache rate', () => {
  render(<ContextControl usage={{ ...fullUsage, estimated: true, avgCacheHitRate: null, cacheHitRate: 0.8 }} />);
  expect(screen.getByText('~20.7K/1M (2%)')).toBeInTheDocument();
  expect(screen.getByText('Average cache hit rate').parentElement).toHaveTextContent('80%');
});

it('skips empty categories and keeps unknown output and cache values blank', () => {
  render(<ContextControl usage={{
    inputTokens: 500, responseReserve: 512, contextLength: 4096,
    breakdown: { messages: 300, systemTools: 150, mcpTools: 0, systemPrompt: 50, skills: 0, metaContext: 0 },
  }} />);
  expect(screen.queryByText('MCP tools')).not.toBeInTheDocument();
  expect(screen.queryByText('Skills')).not.toBeInTheDocument();
  expect(screen.getByText('Messages').parentElement).toHaveTextContent('60%');
  expect(screen.getByText('Output').parentElement).toHaveTextContent('—');
  expect(screen.getByText('Average cache hit rate').parentElement).toHaveTextContent('—');
});

it('stays idle before the first measured turn', () => {
  render(<ContextControl />);
  expect(screen.getByRole('button', { name: 'Context usage details' })).toBeInTheDocument();
  expect(screen.getByText('No usage measured yet')).toBeInTheDocument();
});
