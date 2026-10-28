import { Recordings } from "./recordings";

// 검색창은 제목만 거른다. 전사 내용까지 찾으려면 Search Transcripts를 쓴다
export default function Command() {
  return <Recordings searchable={false} />;
}
