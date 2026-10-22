import { describe, expect, it } from 'vitest';
import { suiteAuthorizeUrl, validateSuiteAuthorizePeer } from './urls.js';

const input = {
  client: 'alpha',
  state: 'request state',
  codeChallenge: 'pkce-challenge',
  hint: 'subject+hint',
  hintDomain: 'shared',
};
const query =
  'client=alpha&state=request+state&code_challenge=pkce-challenge&hint=subject%2Bhint&hint_domain=shared';

describe('suite authorize URLs', () => {
  it('keeps the default endpoint and protocol query unchanged', () => {
    expect(suiteAuthorizeUrl({ webUrl: 'https://beta.example' }, input)).toBe(
      `https://beta.example/suite/authorize?${query}`,
    );
    expect(
      suiteAuthorizeUrl(
        { webUrl: 'https://beta.example' },
        {
          client: 'alpha',
          state: 'state',
          codeChallenge: 'challenge',
        },
      ),
    ).toBe(
      'https://beta.example/suite/authorize?client=alpha&state=state&code_challenge=challenge',
    );
  });

  it.each(['/suite/authorize/beta', '/portal/suite/authorize/beta/'])(
    'preserves the hosted path %s and every parameter',
    (path) => {
      const peer = {
        webUrl: 'https://beta.example',
        authorizeUrl: `https://account.example${path}`,
      };
      expect(() => {
        validateSuiteAuthorizePeer(peer);
      }).not.toThrow();
      expect(suiteAuthorizeUrl(peer, input)).toBe(`https://account.example${path}?${query}`);
    },
  );

  it.each([
    '/suite/authorize/beta',
    '//account.example/suite/authorize/beta',
    'https:account.example/suite/authorize/beta',
    'http://account.example/suite/authorize/beta',
    'ftp://account.example/suite/authorize/beta',
    'https://user:secret@account.example/suite/authorize/beta',
    'https://user@account.example/suite/authorize/beta',
    'https://@account.example/suite/authorize/beta',
    'https://account.example/suite/authorize/beta?existing=value',
    'https://account.example/suite/authorize/beta?',
    'https://account.example/suite/authorize/beta#fragment',
    'https://account.example/suite/authorize/beta#',
    ' https://account.example/suite/authorize/beta',
    'https://account.example/\nsuite/authorize/beta',
  ])('rejects invalid configuration %s before starting', (authorizeUrl) => {
    const peer = { webUrl: 'https://beta.example', authorizeUrl };
    expect(() => {
      validateSuiteAuthorizePeer(peer, { allowLoopback: true });
    }).toThrow(TypeError);
    expect(() => suiteAuthorizeUrl(peer, input, { allowLoopback: true })).toThrow(TypeError);
  });

  it.each(['localhost', '127.0.0.1', '127.5.4.3', '[::1]'])(
    'requires loopback opt-in for %s',
    (host) => {
      const peer = {
        webUrl: 'https://beta.example',
        authorizeUrl: `http://${host}:4100/suite/authorize/beta`,
      };
      expect(() => {
        validateSuiteAuthorizePeer(peer);
      }).toThrow(TypeError);
      expect(suiteAuthorizeUrl(peer, input, { allowLoopback: true })).toBe(
        `${peer.authorizeUrl}?${query}`,
      );
    },
  );
});
