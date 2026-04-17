<script lang="ts">
  import '@fontsource-variable/jetbrains-mono/index.css';
  import '@fontsource-variable/geist/index.css';
  import '../app.css';
  import 'highlight.js/styles/atom-one-dark.css';

  let { children } = $props();
</script>

{@render children()}
