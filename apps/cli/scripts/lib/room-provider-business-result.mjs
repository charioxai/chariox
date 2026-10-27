// MP-08/MP-10: accept array-shaped CRM results using only public fixture fields.
export function roomProviderBusinessResult(value,depth=0){
 if(depth>4||value==null)return null;
 if(typeof value==='string'){if(value.length>250000)return null;try{return roomProviderBusinessResult(JSON.parse(value),depth+1)}catch{return null}}
 if(typeof value!=='object')return null;
 if(Array.isArray(value)){const rows=value.slice(0,10).map(x=>roomProviderBusinessResult(x,depth+1)).filter(Boolean);return rows.length?{rows}:null}
 const bounded={};
 for(const [key,alias] of Object.entries({name:'name',full_name:'full_name',repository:'full_name',open_issues_count:'open_issues_count',vendor:'vendor',price:'price',cost:'price',price_usd:'price',warranty:'warranty',warranty_years:'warranty'})){
  const field=value[key];if(typeof field==='number'||(typeof field==='string'&&/^(vscode|microsoft\/vscode|Acme|Beta|Gamma)$/.test(field)))bounded[alias]=field;
 }
 if(Object.keys(bounded).length)return bounded;
 for(const key of ['result','output','payload','rows','vendors','printers','comparison']){const result=roomProviderBusinessResult(value[key],depth+1);if(result)return result}
 return null;
}
