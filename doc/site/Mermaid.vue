<script setup>
import { ref, onMounted, watch, nextTick } from 'vue';
const props = defineProps({code: String});
const container = ref(null), error = ref('');
let sequence = 0;
async function render() {
  await nextTick();
  if(!container.value) return;
  try {
    const {default: mermaid} = await import('mermaid');
    mermaid.initialize({startOnLoad:false,securityLevel:'strict',theme:'default'});
    const id = 'diagram-' + Math.random().toString(36).slice(2) + '-' + sequence++;
    const result=await mermaid.render(id,props.code);
    container.value.innerHTML=result.svg;
    result.bindFunctions?.(container.value);
  } catch(e) { error.value=String(e); }
}
onMounted(render); watch(()=>props.code,render);
</script>
<template><div class="mermaid-diagram" ref="container" aria-label="Mermaid 图表"></div><pre v-if="error" class="mermaid-error">{{error}}</pre></template>
