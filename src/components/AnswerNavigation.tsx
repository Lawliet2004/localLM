import { useEffect, useState, type RefObject } from 'react';
import { ListTree } from 'lucide-react';
import { prefersReducedMotion } from '../lib/appearance';

/** Read the canonical renderer's headings after completion. No second parser,
 * generated HTML, guessed anchors, or streaming-time Markdown work. */
export function AnswerNavigation({ article, content }: { article: RefObject<HTMLElement | null>; content: string }) {
  const [headings, setHeadings] = useState<HTMLElement[]>([]);
  useEffect(() => {
    setHeadings(content ? Array.from(article.current?.querySelectorAll<HTMLElement>('.markdown h1, .markdown h2, .markdown h3') ?? [])
      .filter(heading => !heading.closest('.reasoning, .trace-thought-details')).slice(0, 40) : []);
  }, [article, content]);
  if (headings.length < 3) return null;
  return <details className="answer-navigation"><summary><ListTree size={14} aria-hidden="true" />In this answer<span>{headings.length} sections</span></summary>
    <nav aria-label="Answer sections"><ol>{headings.map((heading, index) => <li key={index} className={heading.tagName === 'H3' ? 'answer-subsection' : undefined}><button type="button" onClick={() => {
      heading.tabIndex = -1;
      heading.focus({ preventScroll: true });
      heading.scrollIntoView({ block: 'start', behavior: prefersReducedMotion() ? 'instant' : 'smooth' });
    }}>{heading.textContent}</button></li>)}</ol></nav>
  </details>;
}
