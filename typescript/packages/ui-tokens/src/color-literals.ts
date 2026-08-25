/**
 * CSS color keywords recognized as raw colors. Keywords that double as ordinary
 * English words in prop values are handled by requiring a style-like context.
 */
const CSS_COLOR_KEYWORDS = new Set([
  'aliceblue',
  'antiquewhite',
  'aqua',
  'aquamarine',
  'azure',
  'beige',
  'bisque',
  'black',
  'blanchedalmond',
  'blue',
  'blueviolet',
  'brown',
  'burlywood',
  'cadetblue',
  'chartreuse',
  'chocolate',
  'coral',
  'cornflowerblue',
  'cornsilk',
  'crimson',
  'cyan',
  'darkblue',
  'darkcyan',
  'darkgoldenrod',
  'darkgray',
  'darkgreen',
  'darkgrey',
  'darkkhaki',
  'darkmagenta',
  'darkolivegreen',
  'darkorange',
  'darkorchid',
  'darkred',
  'darksalmon',
  'darkseagreen',
  'darkslateblue',
  'darkslategray',
  'darkslategrey',
  'darkturquoise',
  'darkviolet',
  'deeppink',
  'deepskyblue',
  'dimgray',
  'dimgrey',
  'dodgerblue',
  'firebrick',
  'floralwhite',
  'forestgreen',
  'fuchsia',
  'gainsboro',
  'ghostwhite',
  'gold',
  'goldenrod',
  'gray',
  'green',
  'greenyellow',
  'grey',
  'honeydew',
  'hotpink',
  'indianred',
  'indigo',
  'ivory',
  'khaki',
  'lavender',
  'lavenderblush',
  'lawngreen',
  'lemonchiffon',
  'lightblue',
  'lightcoral',
  'lightcyan',
  'lightgoldenrodyellow',
  'lightgray',
  'lightgreen',
  'lightgrey',
  'lightpink',
  'lightsalmon',
  'lightseagreen',
  'lightskyblue',
  'lightslategray',
  'lightslategrey',
  'lightsteelblue',
  'lightyellow',
  'lime',
  'limegreen',
  'linen',
  'magenta',
  'maroon',
  'mediumaquamarine',
  'mediumblue',
  'mediumorchid',
  'mediumpurple',
  'mediumseagreen',
  'mediumslateblue',
  'mediumspringgreen',
  'mediumturquoise',
  'mediumvioletred',
  'midnightblue',
  'mintcream',
  'mistyrose',
  'moccasin',
  'navajowhite',
  'navy',
  'oldlace',
  'olive',
  'olivedrab',
  'orange',
  'orangered',
  'orchid',
  'palegoldenrod',
  'palegreen',
  'paleturquoise',
  'palevioletred',
  'papayawhip',
  'peachpuff',
  'peru',
  'pink',
  'plum',
  'powderblue',
  'purple',
  'rebeccapurple',
  'red',
  'rosybrown',
  'royalblue',
  'saddlebrown',
  'salmon',
  'sandybrown',
  'seagreen',
  'seashell',
  'sienna',
  'silver',
  'skyblue',
  'slateblue',
  'slategray',
  'slategrey',
  'snow',
  'springgreen',
  'steelblue',
  'tan',
  'teal',
  'thistle',
  'tomato',
  'turquoise',
  'violet',
  'wheat',
  'white',
  'whitesmoke',
  'yellow',
  'yellowgreen',
]);

const HEX_PATTERN = /#(?:[0-9a-f]{3,4}|[0-9a-f]{6}|[0-9a-f]{8})\b/i;
const FUNCTIONAL_PATTERN = /\b(?:rgba?|hsla?|hwb|lab|lch|oklab|oklch|color)\s*\(/i;
const KEYWORD_SPLIT_PATTERN = /[^a-z]+/i;

/** Kind of raw color found in a literal, used to pick the report message. */
export type RawColorKind = 'hex' | 'function' | 'keyword';

/** A raw color match with the exact text that triggered it. */
export interface RawColorMatch {
  readonly kind: RawColorKind;
  readonly text: string;
}

function findKeyword(value: string): string | undefined {
  for (const word of value.split(KEYWORD_SPLIT_PATTERN)) {
    if (word !== '' && CSS_COLOR_KEYWORDS.has(word.toLowerCase())) return word;
  }
  return undefined;
}

/**
 * Finds the first raw CSS color in a string. Hex and functional notations are
 * unambiguous anywhere; keyword matches are only meaningful in a style-like
 * context, which the caller decides.
 */
export function findRawColor(value: string, includeKeywords: boolean): RawColorMatch | undefined {
  const hex = HEX_PATTERN.exec(value);
  if (hex) return { kind: 'hex', text: hex[0] };

  const functional = FUNCTIONAL_PATTERN.exec(value);
  if (functional) return { kind: 'function', text: functional[0].replace(/\s*\($/, '') };

  if (!includeKeywords) return undefined;
  const keyword = findKeyword(value);
  return keyword === undefined ? undefined : { kind: 'keyword', text: keyword };
}

/** True when the value is a CSS color keyword on its own. */
export function isCssColorKeyword(value: string): boolean {
  return CSS_COLOR_KEYWORDS.has(value.trim().toLowerCase());
}
