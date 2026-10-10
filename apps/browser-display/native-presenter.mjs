import {attachBrowserDisplay} from './presenter.mjs';

export const nativeMinimumProtocolVersion = 497;
export const nativeRelayPeerProtocolVersion = 106;

// Use the same CXD1 event decoder, credit lifecycle and canvas presenter.
// Only typed request adaptation and native delivery registration differ.
export async function attachNativeDisplay(canvas, transport, target, options = {}) {
  if (!Number.isSafeInteger(transport.kernelProtocolVersion) || typeof transport.subscribeDisplay !== "function" || transport.kernelProtocolVersion < nativeMinimumProtocolVersion || transport.displayEventEncoding !== 'CXD1')
    throw Error('M2 native display requires protocol 497 and CXD1');
  if (!target || typeof target.surface_id !== 'string' || !target.surface_id || typeof target.generation !== 'string' || !target.generation || target.generation.length > 128)
    throw Error('M2 native display requires a current surface/epoch');
  const adapter = {
    kernelProtocolVersion: transport.kernelProtocolVersion,
    relayProtocolVersion: transport.relayProtocolVersion,
    displayEventEncoding: transport.displayEventEncoding,
    onEvent: fn => transport.onEvent(fn),
    subscribeDisplay: binding => transport.subscribeDisplay({...binding,generation:`native:${binding.generation}`}),
    unsubscribeDisplay: binding => transport.unsubscribeDisplay?.({...binding,generation:`native:${binding.generation}`}),
    async request({KernelBrowser:{command}}) {
      let native;
      switch(command.op) {
        case 'display_subscribe': native={op:'display_subscribe',target,codecs:['png']};break;
        case 'display_next': native={op:'display_next',target,subscription_id:command.subscription_id,after_sequence:command.after_sequence,repair:false};break;
        case 'unsubscribe': native={op:'display_close',target,subscription_id:command.subscription_id};break;
        // M2 is an observer. Existing bounded Computer input remains separate.
        default: throw Error('M2 native display operation unavailable');
      }
      return transport.request({KernelBrowser:{command:{op:'computer',command:native}}});
    }
  };
  const stream=await attachBrowserDisplay(canvas,adapter,{...target,display_kind:'native'}, {...options,push:false,codec:'png',stripes:false,creditWindow:1});
  stream.repair=async()=>{
    // A lost base gets a new caller-bound subscription; never replay a credit.
    await stream.close();return attachNativeDisplay(canvas,transport,target,options);
  };
  return stream;
}
