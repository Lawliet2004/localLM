/// <reference types="vite/client" />
import { createRoot } from 'react-dom/client';
import { Execution } from '../src/components/Execution';
import '../src/styles.css';
import '../src/settings.css';

createRoot(document.getElementById('root')!).render(<Execution section="web-search" />);
