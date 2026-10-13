<script setup lang="ts">
defineProps<{ name: string; size?: number }>();
const paths: Record<string, string> = {
  search: 'm21 21-5-5M18 10a8 8 0 1 1-16 0 8 8 0 0 1 16 0',
  upload: 'M12 16V3m-5 5 5-5 5 5M4 15v5h16v-5',
  file: 'M14 2H5v20h14V7l-5-5zm0 0v6h5M8 12h8M8 16h6',
  stream: 'M3 6h3m4 0h11M3 12h3m4 0h11M3 18h3m4 0h11',
  pause: 'M7 4v16M17 4v16', play: 'm7 4 13 8-13 8V4',
  down: 'm6 9 6 6 6-6', up: 'm6 15 6-6 6 6',
  left: 'm14 5-7 7 7 7', right: 'm10 5 7 7-7 7',
  close: 'm6 6 12 12M6 18 18 6',
  follow: 'M5 20h14M12 3v12m-5-5 5 5 5-5',
  history: 'M3 11a9 9 0 1 1 2 7M3 4v7h7M12 7v5l4 2',
  save: 'M4 3h13l4 4v14H3V3h1zm3 0v6h10V3M7 21v-8h10v8',
  download: 'M12 3v12m-5-5 5 5 5-5M4 17v4h16v-4',
  copy: 'M8 8h12v13H8zM16 8V3H3v13h5',
  detail: 'M3 4h18v16H3zM14 4v16M17 8h1m-1 4h1',
  menu: 'M3 5h18M3 12h18M3 19h18',
  sun: 'M12 1v2m0 18v2M1 12h2m18 0h2M4 4l2 2m12 12 2 2M4 20l2-2M18 6l2-2M17 12a5 5 0 1 1-10 0 5 5 0 0 1 10 0',
  moon: 'M20 15A9 9 0 0 1 9 3a9 9 0 1 0 11 12',
  help: 'M9 8a3 3 0 0 1 6 0c0 2-3 2-3 5m0 3v1M22 12a10 10 0 1 1-20 0 10 10 0 0 1 20 0',
  check: 'm5 12 4 4L20 5',
  warning: 'm12 3 10 18H2L12 3zm0 6v5m0 3v1',
  context: 'M3 4h18M3 20h18M3 8h4m10 0h4M3 16h4m10 0h4m-9-4h12m-4-4 4 4-4 4',
  stop: 'M5 5h14v14H5z',
};
</script>
<template><svg :width="size || 16" :height="size || 16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true" focusable="false"><path :d="paths[name] || paths.file" /></svg></template>
