import { fireEvent, render, screen } from '@testing-library/react';
import { expect, it } from 'vitest';
import { ResponseMetrics } from './ResponseMetrics';
import type { Message } from '../lib/types';
const message: Message = { id: '1', conversationId: 'c', role: 'assistant', content: 'answer', reasoning: '', status: 'complete', createdAt: 0 };
it('shows missing legacy timing honestly and expands the measurement boundary', () => {
  render(<ResponseMetrics message={message} />);
  const summary = screen.getByText('Response diagnostics');
  fireEvent.click(summary);
  expect(screen.getByText('This message has no recorded measurements.')).toBeInTheDocument();
  expect(screen.getByText(/before UI rendering/)).toBeInTheDocument();
});
it('displays measured response throughput and first-token time', () => {
  render(<ResponseMetrics message={{ ...message, metrics: { model: 'model.gguf', totalMs: 5820, rounds: [{ complete: true, firstTokenMs: 820, durationMs: 5820, usage: { outputTokens: 123, processedPromptTokens: 385, promptMs: 1000, generationMs: 5000 } }] } }} />);
  expect(screen.getByText('24.6 tok/s')).toBeInTheDocument();
  expect(screen.getByText('385.0 tok/s')).toBeInTheDocument();
  expect(screen.getByText('0.82s')).toBeInTheDocument();
});

it('keeps recorded reasoning and approval phases separate from time to visible answer', () => {
  render(<ResponseMetrics message={{ ...message, metrics: { model: 'fixture', totalMs: 2000, preparationMs: 100, toolsMs: 200, approvalMs: 300, rounds: [{ complete: true, firstTokenMs: 500, firstVisibleAnswerMs: 1200, reasoningMs: 400, durationMs: 1000, usage: {} }] } }} />);
  expect(screen.getByText('Observed reasoning').nextElementSibling).toHaveTextContent('0.40s');
  expect(screen.getByText('Approval wait').nextElementSibling).toHaveTextContent('0.30s');
  expect(screen.getByText('First visible answer').nextElementSibling).toHaveTextContent('1.20s');
});
