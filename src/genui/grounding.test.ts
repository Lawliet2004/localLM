import { describe,it,expect } from 'vitest';
import { parseEvidence,groundedUrls,isGroundedUrl } from './grounding';
describe('evidence and action grounding',()=>{
 it.each(['null','[]','1','true','"text"','bad'])('rejects non-map evidence %s',input=>{expect(parseEvidence(input)).toEqual({});});
 it('rejects invented URLs and canonicalizes actual evidence',()=>{const allowed=groundedUrls('Read https://example.com/docs.',{s1:'https://example.com/source'});expect(isGroundedUrl('https://example.com/docs',allowed)).toBe(true);expect(isGroundedUrl('https://attacker.example',allowed)).toBe(false);expect(groundedUrls('answer',null).size).toBe(0);});
});
