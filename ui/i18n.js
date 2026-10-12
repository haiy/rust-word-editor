import catalog from '../locales/en.json';
const entries=Object.entries(catalog).filter(([a,b])=>a!==b).sort((a,b)=>b[0].length-a[0].length);
export function serviceText(value){
 if(window.__rustWordBoot?.locale!=='en'||typeof value!=='string')return value;
 if(catalog[value])return catalog[value];
 // Service errors/status only: never run this on document text, paths or filenames.
 let result=value;
 for(const [source,target] of entries)if(source.length>1)result=result.split(source).join(target);
 return result;
}
export function serviceData(data){
 if(!data||typeof data!=='object'||Array.isArray(data))return data;
 const result={...data};
 for(const field of ['status','capacity'])if(typeof result[field]==='string')result[field]=serviceText(result[field]);
 if(Array.isArray(result.warnings))result.warnings=result.warnings.map(serviceText);
 return result;
}
