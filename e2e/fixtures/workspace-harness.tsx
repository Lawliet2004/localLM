import { createRoot } from 'react-dom/client';
import { WorkspacePanel } from '../../src/components/WorkspacePanel';
import '../../src/tokens.css';
import '../../src/styles.css';
import '../../src/workspace.css';

document.documentElement.dataset.theme = 'light';

createRoot(document.getElementById('root')!).render(
  <div style={{ position: 'relative', width: '100vw', height: '100vh', background: 'var(--background)' }}>
    <WorkspacePanel busy={false} revision="fixture" />
  </div>,
);
