import { MarkdownView } from '../components/MarkdownView';
import { useEvidence } from './grounding';
import { safeSourceUrl } from '../lib/sources';

/** Share answer formatting while retaining the grounded-link and image policy. */
export function VisualMarkdown({ text }: { text: string }) {
  const evidence = useEvidence();
  return (
    <div className="genui-md">
      <MarkdownView content={text} sourceLinks={JSON.stringify(evidence)}
        allowedLinkUrls={Object.values(evidence).filter(url => safeSourceUrl(url))} allowImages={false} />
    </div>
  );
}
