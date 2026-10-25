import {createFileRoute,useSearch} from "@tanstack/react-router";import {CaptureViewer} from "../capture/CaptureViewer";
export const Route=createFileRoute("/capture")({validateSearch:(s:Record<string,unknown>)=>({stream:typeof s.stream==="string"?s.stream:undefined,title:typeof s.title==="string"?s.title:undefined}),component:CaptureRoute});
function CaptureRoute(){const {stream,title}=useSearch({from:"/capture"}) as {stream?:string;title?:string};return <main style={{position:"fixed",inset:0}}><CaptureViewer streamUrl={stream} title={title??"Capture Viewer"}/></main>}
