import fs from 'node:fs';
const j = JSON.parse(fs.readFileSync('data/shot_07.json', 'utf8'));
function scan(layers, where){
  let source = null;
  for(const l of layers||[]){
    if(l.tt && l.tt!==0){
      console.log(`${where} | target ind=${l.ind} nm=${l.nm} ty=${l.ty} tt=${l.tt} <- source ${source?`ind=${source.ind} nm=${source.nm} ty=${source.ty}`:'NONE'}`);
    } else {
      source = l;
    }
  }
}
scan(j.layers,'root');
(j.assets||[]).forEach(a=>scan(a.layers,'asset '+a.nm));
