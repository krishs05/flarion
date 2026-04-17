import { marked } from 'marked';
import hljs from 'highlight.js';
import DOMPurify from 'dompurify';

marked.setOptions({
  breaks: true,
  gfm: true
});

const renderer = new marked.Renderer();
renderer.code = ({ text, lang }) => {
  const language = lang && hljs.getLanguage(lang) ? lang : 'plaintext';
  const highlighted = hljs.highlight(text, { language }).value;
  return `<pre><code class="hljs language-${language}">${highlighted}</code></pre>`;
};

// Sanitize before `{@html}`: strips scripts, iframes, inline handlers, and
// `javascript:` URLs while keeping normal markdown output.
export function renderMarkdown(content: string): string {
  const raw = marked.parse(content, { renderer }) as string;
  return DOMPurify.sanitize(raw, {
    USE_PROFILES: { html: true }
  });
}
