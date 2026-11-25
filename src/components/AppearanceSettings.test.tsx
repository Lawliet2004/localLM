import { cleanup, fireEvent, render, screen, within } from '@testing-library/react';
import { afterEach, expect, it, vi } from 'vitest';
import '../styles.css';
import { APPEARANCE_STORAGE_KEY, defaultAppearance } from '../lib/appearance';
import { useAppearance } from '../lib/useAppearance';
import { AppearanceSettings } from './AppearanceSettings';

function Settings() {
  const appearance = useAppearance();
  return <AppearanceSettings preferences={appearance.preferences} theme={appearance.theme}
    onChange={appearance.update} onReset={appearance.reset} saveError={appearance.saveError} />;
}

afterEach(() => {
  cleanup();
  localStorage.clear();
  document.documentElement.removeAttribute('style');
  for (const key of Object.keys(document.documentElement.dataset)) delete document.documentElement.dataset[key];
  vi.unstubAllGlobals();
});

it('previews syntax overrides immediately, preserves both variants and restores defaults', () => {
  const root = document.documentElement;
  const style = () => root.style.getPropertyValue('--syn-keyword');
  const editKeyword = (value: string) => {
    const input = screen.getByLabelText('Keywords', { selector: 'input[type=text]' });
    fireEvent.change(input, { target: { value } });
    fireEvent.blur(input);
  };
  const first = render(<Settings />);
  fireEvent.change(screen.getByLabelText('Base palette'), { target: { value: 'indigo' } });
  expect(root).toHaveAttribute('data-palette', 'indigo');
  const preview = within(screen.getByRole('region', { name: 'Live theme preview' }));
  expect(preview.getByLabelText('Syntax color preview').querySelector('.hljs-keyword')).toBeTruthy();
  fireEvent.change(preview.getByLabelText('Code preview'), { target: { value: 'python' } });
  expect(preview.getByLabelText('python source')).toHaveTextContent('def greet');

  fireEvent.click(screen.getByRole('button', { name: 'Syntax highlighting' }));
  editKeyword('invalid');
  expect(screen.getByText('Use # followed by six hex digits.')).toBeInTheDocument();
  expect(style()).toBe('');
  editKeyword('#FF0000');
  expect(style()).toBe('#ff0000');

  fireEvent.click(screen.getByRole('button', { name: 'Use dark appearance' }));
  expect(style()).toBe('');
  editKeyword('#ffccaa');
  fireEvent.click(screen.getByRole('button', { name: 'Use light appearance' }));
  expect(style()).toBe('#ff0000');

  first.unmount();
  render(<Settings />);
  expect(style()).toBe('#ff0000');
  fireEvent.click(screen.getByRole('button', { name: 'Reset light colors' }));
  expect(style()).toBe('');
  fireEvent.click(screen.getByRole('button', { name: 'Use dark appearance' }));
  expect(style()).toBe('#ffccaa');
  fireEvent.click(screen.getByRole('button', { name: 'Reset appearance' }));
  expect(style()).toBe('');
  expect(JSON.parse(localStorage.getItem(APPEARANCE_STORAGE_KEY)!)).toEqual(defaultAppearance);
});

it('replays the sample at the selected speed and respects reduced motion', () => {
  render(<Settings />);
  const firstCard = screen.getByText('Workspace ready').closest('.appearance-motion-card');
  fireEvent.click(screen.getByRole('button', { name: 'Replay animation' }));
  expect(firstCard).not.toBeInTheDocument();
  fireEvent.change(screen.getByLabelText('Animation speed'), { target: { value: 'slow' } });
  expect(document.documentElement).toHaveAttribute('data-motion-speed', 'slow');
  fireEvent.change(screen.getByLabelText('Animations'), { target: { value: 'reduced' } });
  expect(screen.getByRole('button', { name: 'Replay animation' })).toBeDisabled();
  expect(screen.getByText('Workspace ready').closest('.appearance-motion-card')).not.toHaveClass('is-animated');
  expect(JSON.parse(localStorage.getItem(APPEARANCE_STORAGE_KEY)!)).toMatchObject({ motion: 'reduced', motionSpeed: 'slow' });
});

it('does not play motion when the operating system asks to reduce it', () => {
  vi.stubGlobal('matchMedia', (query: string) => ({
    matches: query.includes('reduced-motion'), addEventListener: vi.fn(), removeEventListener: vi.fn(),
  }));
  render(<Settings />);
  expect(screen.getByRole('button', { name: 'Replay animation' })).toBeDisabled();
});

it('applies a bundled home wallpaper, restores it on reopen and supports clearing it', () => {
  const first = render(<Settings />);
  fireEvent.click(screen.getByRole('button', { name: 'Cherry blossom' }));
  expect(document.documentElement).toHaveAttribute('data-home-wallpaper', 'sakura');
  expect(document.documentElement.style.getPropertyValue('--home-wallpaper')).toContain('sakura');
  first.unmount();

  render(<Settings />);
  expect(screen.getByRole('button', { name: 'Cherry blossom' })).toHaveAttribute('aria-pressed', 'true');
  fireEvent.click(screen.getByRole('button', { name: 'None' }));
  expect(document.documentElement).toHaveAttribute('data-home-wallpaper', 'none');
  expect(document.documentElement.style.getPropertyValue('--home-wallpaper')).toBe('');
});
