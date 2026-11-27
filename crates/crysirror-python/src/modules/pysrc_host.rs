// SPDX-License-Identifier: MPL-2.0
//! Python-source standard library modules whose CPython versions reach
//! the network, child processes or the window system. The embedding host
//! provides none of these, so their entry points raise the errors CPython
//! raises when the facility is unavailable (`URLError`, `OSError`), and the
//! pure parts (URL parsing, archive formats over `zlib`) follow the
//! documented behaviour of the CPython modules.

pub fn is_package(name: &str) -> bool {
    matches!(name, "urllib" | "encodings" | "email" | "xml" | "html")
}

pub fn source(name: &str) -> Option<&'static str> {
    Some(match name {
        "urllib" => URLLIB,
        "urllib.parse" | "urlparse" => URLLIB_PARSE,
        "urllib.error" => URLLIB_ERROR,
        "urllib.request" | "urllib2" => URLLIB_REQUEST,
        "subprocess" => SUBPROCESS,
        "tarfile" => TARFILE,
        "zipfile" => ZIPFILE,
        "gzip" => GZIP,
        "webbrowser" => "def open(url, new=0, autoraise=True):\n    return False\ndef open_new(url):\n    return False\ndef open_new_tab(url):\n    return False\nclass Error(Exception):\n    pass\ndef get(using=None):\n    raise Error('could not locate runnable browser')\n",
        "socket" => SOCKET,
        "select" => "class error(OSError):\n    pass\ndef select(rlist, wlist, xlist, timeout=None):\n    raise OSError('select is not available')\n",
        "signal" => SIGNAL,
        "encodings" | "email" | "xml" => "",
        "encodings.latin_1" | "encodings.utf_8" | "encodings.ascii" => "import codecs\n",
        "html" => HTML,
        "ast" => AST,
        _ => return None,
    })
}

const URLLIB: &str = r##"
import sys
if sys.version_info[0] < 3:
    from urllib.parse import quote, quote_plus, unquote, unquote_plus, urlencode
    from urllib.request import urlopen, urlretrieve
"##;

const URLLIB_PARSE: &str = r##"
import collections

_hexdig = '0123456789ABCDEFabcdef'
_always_safe = frozenset('ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789_.-~')
uses_relative = ['', 'ftp', 'http', 'gopher', 'nntp', 'imap', 'wais', 'file', 'https', 'shttp', 'mms', 'prospero', 'rtsp', 'rtspu', 'sftp', 'svn', 'svn+ssh', 'ws', 'wss']
uses_netloc = ['', 'ftp', 'http', 'gopher', 'nntp', 'telnet', 'imap', 'wais', 'file', 'mms', 'https', 'shttp', 'snews', 'prospero', 'rtsp', 'rtspu', 'rsync', 'svn', 'svn+ssh', 'sftp', 'nfs', 'git', 'git+ssh', 'ws', 'wss']
uses_params = ['', 'ftp', 'hdl', 'prospero', 'http', 'imap', 'https', 'shttp', 'rtsp', 'rtspu', 'sip', 'sips', 'mms', 'sftp', 'tel']
scheme_chars = 'abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789+-.'


class _NetlocMixin(object):
    @property
    def username(self):
        userinfo, have, _ = self.netloc.rpartition('@')
        if not have:
            return None
        return userinfo.partition(':')[0]

    @property
    def password(self):
        userinfo, have, _ = self.netloc.rpartition('@')
        if not have:
            return None
        u, have_pw, pw = userinfo.partition(':')
        return pw if have_pw else None

    @property
    def hostname(self):
        host = self.netloc.rpartition('@')[2]
        if host.startswith('['):
            h = host[1:].partition(']')[0]
        else:
            h = host.partition(':')[0]
        if not h:
            return None
        return h.lower()

    @property
    def port(self):
        host = self.netloc.rpartition('@')[2]
        if host.startswith('['):
            host = host.partition(']')[2]
            _, have, port = host.partition(':')
        else:
            _, have, port = host.partition(':')
        if not have or not port:
            return None
        p = int(port, 10)
        if not (0 <= p <= 65535):
            raise ValueError('Port out of range 0-65535')
        return p


class SplitResult(collections.namedtuple('SplitResult', 'scheme netloc path query fragment'), _NetlocMixin):
    __slots__ = ()

    def geturl(self):
        return urlunsplit(self)


class ParseResult(collections.namedtuple('ParseResult', 'scheme netloc path params query fragment'), _NetlocMixin):
    __slots__ = ()

    def geturl(self):
        return urlunparse(self)


def _splitnetloc(url, start=0):
    delim = len(url)
    for c in '/?#':
        wdelim = url.find(c, start)
        if wdelim >= 0:
            delim = min(delim, wdelim)
    return url[start:delim], url[delim:]


def urlsplit(url, scheme='', allow_fragments=True):
    netloc = query = fragment = ''
    i = url.find(':')
    if i > 0 and url[0].isascii() and url[0].isalpha():
        for c in url[:i]:
            if c not in scheme_chars:
                break
        else:
            scheme, url = url[:i].lower(), url[i + 1:]
    if url[:2] == '//':
        netloc, url = _splitnetloc(url, 2)
        if (('[' in netloc and ']' not in netloc) or (']' in netloc and '[' not in netloc)):
            raise ValueError('Invalid IPv6 URL')
    if allow_fragments and '#' in url:
        url, fragment = url.split('#', 1)
    if '?' in url:
        url, query = url.split('?', 1)
    return SplitResult(scheme, netloc, url, query, fragment)


def _splitparams(url):
    if '/' in url:
        i = url.find(';', url.rfind('/'))
        if i < 0:
            return url, ''
    else:
        i = url.find(';')
    return url[:i], url[i + 1:]


def urlparse(url, scheme='', allow_fragments=True):
    splitresult = urlsplit(url, scheme, allow_fragments)
    scheme, netloc, url, query, fragment = splitresult
    if scheme in uses_params and ';' in url:
        url, params = _splitparams(url)
    else:
        params = ''
    return ParseResult(scheme, netloc, url, params, query, fragment)


def urlunsplit(components):
    scheme, netloc, url, query, fragment = components
    if netloc or (scheme and scheme in uses_netloc and url[:2] != '//'):
        if url and url[:1] != '/':
            url = '/' + url
        url = '//' + (netloc or '') + url
    if scheme:
        url = scheme + ':' + url
    if query:
        url = url + '?' + query
    if fragment:
        url = url + '#' + fragment
    return url


def urlunparse(components):
    scheme, netloc, url, params, query, fragment = components
    if params:
        url = "%s;%s" % (url, params)
    return urlunsplit((scheme, netloc, url, query, fragment))


def urljoin(base, url, allow_fragments=True):
    if not base:
        return url
    if not url:
        return base
    bscheme, bnetloc, bpath, bparams, bquery, bfragment = urlparse(base, '', allow_fragments)
    scheme, netloc, path, params, query, fragment = urlparse(url, bscheme, allow_fragments)
    if not scheme:
        scheme = bscheme
    if scheme != bscheme or scheme not in uses_relative:
        return url
    if scheme in uses_netloc:
        if netloc:
            return urlunparse((scheme, netloc, path, params, query, fragment))
        netloc = bnetloc
    if not path and not params:
        path = bpath
        params = bparams
        if not query:
            query = bquery
        return urlunparse((scheme, netloc, path, params, query, fragment))
    base_parts = bpath.split('/')
    if base_parts[-1] != '':
        del base_parts[-1]
    if path[:1] == '/':
        segments = path.split('/')
    else:
        segments = base_parts + path.split('/')
        segments[1:-1] = filter(None, segments[1:-1])
    resolved_path = []
    for seg in segments:
        if seg == '..':
            try:
                resolved_path.pop()
            except IndexError:
                pass
        elif seg == '.':
            continue
        else:
            resolved_path.append(seg)
    if segments[-1] in ('.', '..'):
        resolved_path.append('')
    return urlunparse((scheme, netloc, '/'.join(resolved_path) or '/', params, query, fragment))


def urldefrag(url):
    if '#' in url:
        s, n, p, a, q, frag = urlparse(url)
        defrag = urlunparse((s, n, p, a, q, ''))
    else:
        frag = ''
        defrag = url
    return defrag, frag


def _to_bytes(s, encoding, errors):
    if isinstance(s, str):
        return s.encode(encoding or 'utf-8', errors or 'strict')
    return bytes(s)


def quote(string, safe='/', encoding=None, errors=None):
    if isinstance(safe, str):
        safe = safe.encode('ascii', 'ignore')
    bs = _to_bytes(string, encoding, errors)
    safe_chars = set(bytes(safe)) | set(c.encode('ascii')[0] for c in _always_safe)
    out = []
    for b in bytearray(bs):
        if b in safe_chars:
            out.append(chr(b))
        else:
            out.append('%%%02X' % b)
    return ''.join(out)


def quote_plus(string, safe='', encoding=None, errors=None):
    if (isinstance(string, str) and ' ' not in string) or (isinstance(string, bytes) and b' ' not in string):
        return quote(string, safe, encoding, errors)
    if isinstance(safe, str):
        space = ' '
    else:
        space = b' '
    string = quote(string, safe + space, encoding, errors)
    return string.replace(' ', '+')


def unquote_to_bytes(string):
    if isinstance(string, str):
        string = string.encode('utf-8')
    bits = bytes(string).split(b'%')
    res = bytearray(bits[0])
    for item in bits[1:]:
        h = item[:2]
        try:
            if len(h) == 2:
                res.append(int(h.decode('ascii'), 16))
                res.extend(item[2:])
                continue
        except ValueError:
            pass
        res.extend(b'%')
        res.extend(item)
    return bytes(res)


def unquote(string, encoding='utf-8', errors='replace'):
    if isinstance(string, bytes):
        return unquote_to_bytes(string).decode(encoding, errors)
    if '%' not in string:
        return string
    return unquote_to_bytes(string).decode(encoding, errors)


def unquote_plus(string, encoding='utf-8', errors='replace'):
    string = string.replace('+', ' ')
    return unquote(string, encoding, errors)


def urlencode(query, doseq=False, safe='', encoding=None, errors=None, quote_via=quote_plus):
    if hasattr(query, "items"):
        query = query.items()
    l = []
    for k, v in query:
        if isinstance(k, bytes):
            k = quote_via(k, safe)
        else:
            k = quote_via(str(k), safe, encoding, errors)
        if not doseq:
            if isinstance(v, bytes):
                v = quote_via(v, safe)
            else:
                v = quote_via(str(v), safe, encoding, errors)
            l.append(k + '=' + v)
        elif isinstance(v, (str, bytes)):
            l.append(k + '=' + quote_via(v, safe, encoding, errors))
        else:
            try:
                x = len(v)
            except TypeError:
                l.append(k + '=' + quote_via(str(v), safe, encoding, errors))
            else:
                for elt in v:
                    l.append(k + '=' + quote_via(elt if isinstance(elt, (str, bytes)) else str(elt), safe, encoding, errors))
    return '&'.join(l)


def parse_qsl(qs, keep_blank_values=False, strict_parsing=False, encoding='utf-8', errors='replace', max_num_fields=None, separator='&'):
    r = []
    for name_value in qs.split(separator):
        if not name_value and not strict_parsing:
            continue
        nv = name_value.split('=', 1)
        if len(nv) != 2:
            if strict_parsing:
                raise ValueError("bad query field: %r" % (name_value,))
            if keep_blank_values:
                nv.append('')
            else:
                continue
        if len(nv[1]) or keep_blank_values:
            name = unquote(nv[0].replace('+', ' '), encoding, errors)
            value = unquote(nv[1].replace('+', ' '), encoding, errors)
            r.append((name, value))
    return r


def parse_qs(qs, keep_blank_values=False, strict_parsing=False, encoding='utf-8', errors='replace', max_num_fields=None, separator='&'):
    parsed_result = {}
    for name, value in parse_qsl(qs, keep_blank_values, strict_parsing, encoding, errors, max_num_fields, separator):
        if name in parsed_result:
            parsed_result[name].append(value)
        else:
            parsed_result[name] = [value]
    return parsed_result
"##;

const URLLIB_ERROR: &str = r##"
class URLError(OSError):
    def __init__(self, reason, filename=None):
        self.args = (reason,)
        self.reason = reason
        if filename is not None:
            self.filename = filename

    def __str__(self):
        return '<urlopen error %s>' % self.reason


class HTTPError(URLError):
    def __init__(self, url, code, msg, hdrs, fp):
        self.code = code
        self.msg = msg
        self.hdrs = hdrs
        self.fp = fp
        self.filename = url
        self.reason = msg

    def __str__(self):
        return 'HTTP Error %s: %s' % (self.code, self.msg)


class ContentTooShortError(URLError):
    def __init__(self, message, content):
        URLError.__init__(self, message)
        self.content = content
"##;

const URLLIB_REQUEST: &str = r##"
from urllib.error import URLError, HTTPError


def _unavailable(url):
    return URLError("network access is not available in this runtime")


class Request(object):
    def __init__(self, url, data=None, headers={}, origin_req_host=None, unverifiable=False, method=None):
        self.full_url = url
        self.data = data
        self.headers = dict(headers)
        self.method = method

    def add_header(self, key, val):
        self.headers[key.capitalize()] = val

    def get_full_url(self):
        return self.full_url

    def get_method(self):
        if self.method is not None:
            return self.method
        return "POST" if self.data is not None else "GET"


def urlopen(url, data=None, timeout=None, *args, **kwargs):
    raise _unavailable(url)


def urlretrieve(url, filename=None, reporthook=None, data=None):
    raise _unavailable(url)


def build_opener(*handlers):
    class _Opener(object):
        def open(self, url, data=None, timeout=None):
            raise _unavailable(url)
    return _Opener()


def install_opener(opener):
    pass


def pathname2url(p):
    from urllib.parse import quote
    return quote(p)


def url2pathname(p):
    from urllib.parse import unquote
    return unquote(p)
"##;

const SUBPROCESS: &str = r##"
PIPE = -1
STDOUT = -2
DEVNULL = -3


class SubprocessError(Exception):
    pass


class CalledProcessError(SubprocessError):
    def __init__(self, returncode, cmd, output=None, stderr=None):
        self.returncode = returncode
        self.cmd = cmd
        self.output = output
        self.stdout = output
        self.stderr = stderr

    def __str__(self):
        return "Command '%s' returned non-zero exit status %d." % (self.cmd, self.returncode)


class TimeoutExpired(SubprocessError):
    def __init__(self, cmd, timeout, output=None, stderr=None):
        self.cmd = cmd
        self.timeout = timeout
        self.output = output
        self.stderr = stderr


def _unavailable(args):
    return OSError(2, "Child processes are not available in this runtime", str(args))


class Popen(object):
    def __init__(self, args, *a, **kw):
        raise _unavailable(args)


def call(args, *a, **kw):
    raise _unavailable(args)


def check_call(args, *a, **kw):
    raise _unavailable(args)


def check_output(args, *a, **kw):
    raise _unavailable(args)


def run(args, *a, **kw):
    raise _unavailable(args)


def getoutput(cmd):
    raise _unavailable(cmd)


def getstatusoutput(cmd):
    raise _unavailable(cmd)
"##;

const TARFILE: &str = r##"
import io, os

NUL = b"\0"
BLOCKSIZE = 512
RECORDSIZE = BLOCKSIZE * 20
REGTYPE = b"0"
AREGTYPE = b"\0"
DIRTYPE = b"5"
SYMTYPE = b"2"
LNKTYPE = b"1"
USTAR_FORMAT = 0
GNU_FORMAT = 1
PAX_FORMAT = 2
DEFAULT_FORMAT = PAX_FORMAT


class TarError(Exception):
    pass


class ReadError(TarError):
    pass


class CompressionError(TarError):
    pass


class StreamError(TarError):
    pass


class ExtractError(TarError):
    pass


class HeaderError(TarError):
    pass


def _nts(s):
    p = s.find(b"\0")
    if p != -1:
        s = s[:p]
    return s.decode("utf-8", "surrogateescape")


def _nti(s):
    s = _nts(s).strip() or "0"
    return int(s, 8)


def _itn(n, digits):
    s = ("%0*o" % (digits - 1, n)).encode("ascii") + NUL
    return s


class TarInfo(object):
    def __init__(self, name=""):
        self.name = name
        self.mode = 0o644
        self.uid = 0
        self.gid = 0
        self.size = 0
        self.mtime = 0
        self.chksum = 0
        self.type = REGTYPE
        self.linkname = ""
        self.uname = ""
        self.gname = ""
        self.offset_data = 0

    def isfile(self):
        return self.type in (REGTYPE, AREGTYPE)

    isreg = isfile

    def isdir(self):
        return self.type == DIRTYPE

    def issym(self):
        return self.type == SYMTYPE

    def islnk(self):
        return self.type == LNKTYPE

    def tobuf(self, format=DEFAULT_FORMAT, encoding="utf-8", errors="surrogateescape"):
        name = self.name.encode(encoding, errors)
        if len(name) > 99:
            raise ValueError("name is too long")
        parts = [
            name.ljust(100, NUL),
            _itn(self.mode & 0o7777, 8),
            _itn(self.uid, 8),
            _itn(self.gid, 8),
            _itn(self.size, 12),
            _itn(int(self.mtime), 12),
            b"        ",
            self.type,
            self.linkname.encode(encoding, errors).ljust(100, NUL),
            b"ustar\x0000",
            self.uname.encode(encoding, errors).ljust(32, NUL),
            self.gname.encode(encoding, errors).ljust(32, NUL),
            _itn(0, 8),
            _itn(0, 8),
            NUL * 155,
        ]
        buf = b"".join(parts).ljust(BLOCKSIZE, NUL)
        chksum = 256 + sum(bytearray(buf[:148])) + sum(bytearray(buf[156:]))
        buf = buf[:148] + ("%06o\0" % chksum).encode("ascii") + buf[155:]
        return buf

    @classmethod
    def frombuf(cls, buf, encoding="utf-8", errors="surrogateescape"):
        if len(buf) != BLOCKSIZE:
            raise HeaderError("truncated header")
        if buf.count(NUL) == BLOCKSIZE:
            raise HeaderError("end of file header")
        chksum = _nti(buf[148:156])
        if chksum not in (256 + sum(bytearray(buf[:148])) + sum(bytearray(buf[156:])),):
            raise HeaderError("bad checksum")
        obj = cls()
        obj.name = _nts(buf[0:100])
        obj.mode = _nti(buf[100:108])
        obj.uid = _nti(buf[108:116])
        obj.gid = _nti(buf[116:124])
        obj.size = _nti(buf[124:136])
        obj.mtime = _nti(buf[136:148])
        obj.chksum = chksum
        obj.type = buf[156:157]
        obj.linkname = _nts(buf[157:257])
        obj.uname = _nts(buf[265:297])
        obj.gname = _nts(buf[297:329])
        prefix = _nts(buf[345:500])
        if prefix and obj.type not in (b"L", b"K"):
            obj.name = prefix + "/" + obj.name
        return obj


class TarFile(object):
    def __init__(self, name=None, mode="r", fileobj=None, **kwargs):
        if fileobj is None:
            fileobj = io.open(name, {"r": "rb", "w": "wb", "a": "r+b", "x": "xb"}[mode[:1]])
            self._extfileobj = False
        else:
            self._extfileobj = True
        self.fileobj = fileobj
        self.name = name
        self.mode = mode[:1]
        self.members = []
        self.closed = False
        self.offset = 0
        if self.mode == "r":
            self._load()

    def _load(self):
        while True:
            buf = self.fileobj.read(BLOCKSIZE)
            if len(buf) < BLOCKSIZE:
                break
            try:
                info = TarInfo.frombuf(buf)
            except HeaderError:
                break
            self.offset += BLOCKSIZE
            info.offset_data = self.offset
            size = (info.size + BLOCKSIZE - 1) // BLOCKSIZE * BLOCKSIZE
            self.fileobj.seek(size, 1)
            self.offset += size
            self.members.append(info)

    def getmembers(self):
        return list(self.members)

    def getnames(self):
        return [m.name for m in self.members]

    def getmember(self, name):
        for m in reversed(self.members):
            if m.name == name:
                return m
        raise KeyError("filename %r not found" % name)

    def extractfile(self, member):
        if isinstance(member, str):
            member = self.getmember(member)
        if not member.isfile():
            return None
        self.fileobj.seek(member.offset_data)
        return io.BytesIO(self.fileobj.read(member.size))

    def addfile(self, tarinfo, fileobj=None):
        self.fileobj.write(tarinfo.tobuf())
        if fileobj is not None:
            data = fileobj.read(tarinfo.size)
            self.fileobj.write(data)
            rem = len(data) % BLOCKSIZE
            if rem:
                self.fileobj.write(NUL * (BLOCKSIZE - rem))
        self.members.append(tarinfo)

    def gettarinfo(self, name=None, arcname=None, fileobj=None):
        raise TarError("gettarinfo is not available in this runtime")

    def add(self, name, arcname=None, recursive=True, filter=None):
        raise TarError("adding files from the file system is not available in this runtime")

    def extract(self, member, path="", set_attrs=True, numeric_owner=False):
        raise ExtractError("extracting to the file system is not available in this runtime")

    def extractall(self, path=".", members=None, numeric_owner=False):
        raise ExtractError("extracting to the file system is not available in this runtime")

    def __iter__(self):
        return iter(self.members)

    def close(self):
        if self.closed:
            return
        if self.mode in "aw":
            self.fileobj.write(NUL * (BLOCKSIZE * 2))
        if not self._extfileobj:
            self.fileobj.close()
        self.closed = True

    def __enter__(self):
        return self

    def __exit__(self, *a):
        self.close()


def open(name=None, mode="r", fileobj=None, bufsize=10240, **kwargs):
    kind = mode.partition(":")[2] or mode.partition("|")[2]
    if kind not in ("", "*"):
        raise CompressionError("compression %r is not supported" % kind)
    return TarFile(name, mode, fileobj, **kwargs)


def is_tarfile(name):
    try:
        t = open(name)
        t.close()
        return bool(t.members)
    except Exception:
        return False
"##;

const ZIPFILE: &str = r##"
import io, struct, zlib, time

ZIP_STORED = 0
ZIP_DEFLATED = 8
ZIP_BZIP2 = 12
ZIP_LZMA = 14


_CP437_HIGH = 'ÇüéâäàåçêëèïîìÄÅÉæÆôöòûùÿÖÜ¢£¥₧ƒáíóúñÑªº¿⌐¬½¼¡«»░▒▓│┤╡╢╖╕╣║╗╝╜╛┐└┴┬├─┼╞╟╚╔╩╦╠═╬╧╨╤╥╙╘╒╓╫╪┘┌█▄▌▐▀αßΓπΣσµτΦΘΩδ∞φε∩≡±≥≤⌠⌡÷≈°∙·√ⁿ²■\xa0'


def _cp437(b):
    return "".join(chr(c) if c < 128 else _CP437_HIGH[c - 128] for c in bytearray(b))


class BadZipFile(Exception):
    pass


BadZipfile = BadZipFile
error = BadZipFile


class LargeZipFile(Exception):
    pass


class ZipInfo(object):
    def __init__(self, filename="NoName", date_time=(1980, 1, 1, 0, 0, 0)):
        self.filename = filename.replace("\\", "/")
        self.orig_filename = filename
        self.date_time = date_time
        self.compress_type = ZIP_STORED
        self.comment = b""
        self.extra = b""
        self.create_system = 3
        self.create_version = 20
        self.extract_version = 20
        self.reserved = 0
        self.flag_bits = 0
        self.volume = 0
        self.internal_attr = 0
        self.external_attr = 0
        self.header_offset = 0
        self.CRC = 0
        self.compress_size = 0
        self.file_size = 0

    def is_dir(self):
        return self.filename.endswith("/")

    def _dostime(self):
        y, mo, d, h, mi, s = self.date_time
        return ((y - 1980) << 9 | mo << 5 | d), (h << 11 | mi << 5 | (s // 2))


class ZipExtFile(io.BytesIO):
    def __init__(self, data, info):
        io.BytesIO.__init__(self, data)
        self.name = info.filename


class ZipFile(object):
    def __init__(self, file, mode="r", compression=ZIP_STORED, allowZip64=True, compresslevel=None, **kw):
        self.mode = mode
        self.compression = compression
        self.compresslevel = compresslevel
        self.filelist = []
        self.NameToInfo = {}
        self.comment = b""
        if isinstance(file, (str, bytes)):
            self.filename = file
            self.fp = io.open(file, {"r": "rb", "w": "wb", "a": "r+b", "x": "xb"}[mode])
            self._filePassed = False
        else:
            self.filename = getattr(file, "name", None)
            self.fp = file
            self._filePassed = True
        self._start = 0
        if mode == "r":
            self._read_directory()
        elif mode == "a":
            try:
                self._read_directory()
                self.fp.seek(self._start_dir)
            except BadZipFile:
                self.fp.seek(0, 2)
        else:
            self._start_dir = 0

    def _read_directory(self):
        fp = self.fp
        fp.seek(0, 2)
        size = fp.tell()
        tail = min(size, 65536 + 22)
        fp.seek(size - tail)
        data = fp.read()
        pos = data.rfind(b"PK\x05\x06")
        if pos < 0:
            raise BadZipFile("File is not a zip file")
        (_, _, _, _, count, dir_size, dir_offset, clen) = struct.unpack("<4sHHHHIIH", data[pos:pos + 22])
        self.comment = data[pos + 22:pos + 22 + clen]
        concat = (size - tail + pos) - dir_size - dir_offset
        self._start_dir = dir_offset + concat
        fp.seek(self._start_dir)
        directory = fp.read(dir_size)
        p = 0
        while p + 46 <= len(directory):
            fields = struct.unpack("<4sBBBBHHHHIIIHHHHHII", directory[p:p + 46])
            if fields[0] != b"PK\x01\x02":
                raise BadZipFile("Bad magic number for central directory")
            nlen, elen, clen = fields[12], fields[13], fields[14]
            name = directory[p + 46:p + 46 + nlen]
            flags = fields[5]
            name = name.decode("utf-8") if flags & 0x800 else _cp437(name)
            x = ZipInfo(name)
            x.extra = directory[p + 46 + nlen:p + 46 + nlen + elen]
            x.comment = directory[p + 46 + nlen + elen:p + 46 + nlen + elen + clen]
            x.create_version, x.create_system, x.extract_version, x.reserved = fields[1], fields[2], fields[3], fields[4]
            x.flag_bits, x.compress_type = flags, fields[6]
            t, d = fields[7], fields[8]
            x.date_time = ((d >> 9) + 1980, (d >> 5) & 0xF, d & 0x1F, t >> 11, (t >> 5) & 0x3F, (t & 0x1F) * 2)
            x.CRC, x.compress_size, x.file_size = fields[9], fields[10], fields[11]
            x.volume, x.internal_attr, x.external_attr = fields[15], fields[16], fields[17]
            x.header_offset = fields[18] + concat
            self.filelist.append(x)
            self.NameToInfo[x.filename] = x
            p += 46 + nlen + elen + clen

    def namelist(self):
        return [i.filename for i in self.filelist]

    def infolist(self):
        return list(self.filelist)

    def getinfo(self, name):
        info = self.NameToInfo.get(name)
        if info is None:
            raise KeyError("There is no item named %r in the archive" % name)
        return info

    def read(self, name, pwd=None):
        info = name if isinstance(name, ZipInfo) else self.getinfo(name)
        self.fp.seek(info.header_offset)
        h = self.fp.read(30)
        if h[:4] != b"PK\x03\x04":
            raise BadZipFile("Bad magic number for file header")
        nlen, elen = struct.unpack("<HH", h[26:30])
        self.fp.seek(info.header_offset + 30 + nlen + elen)
        data = self.fp.read(info.compress_size)
        if info.compress_type == ZIP_STORED:
            pass
        elif info.compress_type == ZIP_DEFLATED:
            data = zlib.decompress(data, -15)
        else:
            raise NotImplementedError("That compression method is not supported")
        if zlib.crc32(data) & 0xffffffff != info.CRC:
            raise BadZipFile("Bad CRC-32 for file %r" % info.filename)
        return data

    def open(self, name, mode="r", pwd=None, **kw):
        if mode != "r":
            raise ValueError("writing members through open() is not supported")
        info = name if isinstance(name, ZipInfo) else self.getinfo(name)
        return ZipExtFile(self.read(info), info)

    def testzip(self):
        for i in self.filelist:
            try:
                self.read(i)
            except BadZipFile:
                return i.filename
        return None

    def writestr(self, zinfo_or_arcname, data, compress_type=None, compresslevel=None):
        if isinstance(data, str):
            data = data.encode("utf-8")
        if isinstance(zinfo_or_arcname, ZipInfo):
            zinfo = zinfo_or_arcname
        else:
            zinfo = ZipInfo(zinfo_or_arcname, time.localtime(time.time())[:6])
            zinfo.compress_type = self.compression
            zinfo.external_attr = 0o600 << 16
        if compress_type is not None:
            zinfo.compress_type = compress_type
        zinfo.file_size = len(data)
        zinfo.CRC = zlib.crc32(data) & 0xffffffff
        if zinfo.compress_type == ZIP_DEFLATED:
            c = zlib.compressobj(compresslevel if compresslevel is not None else (self.compresslevel if self.compresslevel is not None else -1), zlib.DEFLATED, -15)
            body = c.compress(data) + c.flush()
        elif zinfo.compress_type == ZIP_STORED:
            body = data
        else:
            raise NotImplementedError("That compression method is not supported")
        zinfo.compress_size = len(body)
        zinfo.header_offset = self.fp.tell()
        name = zinfo.filename.encode("utf-8")
        flags = zinfo.flag_bits | (0x800 if any(ord(c) > 127 for c in zinfo.filename) else 0)
        zinfo.flag_bits = flags
        d, t = zinfo._dostime()
        self.fp.write(struct.pack("<4sHHHHHIIIHH", b"PK\x03\x04", zinfo.extract_version, flags, zinfo.compress_type, t, d, zinfo.CRC, zinfo.compress_size, zinfo.file_size, len(name), len(zinfo.extra)))
        self.fp.write(name)
        self.fp.write(zinfo.extra)
        self.fp.write(body)
        self.filelist.append(zinfo)
        self.NameToInfo[zinfo.filename] = zinfo

    def write(self, filename, arcname=None, compress_type=None, compresslevel=None):
        with io.open(filename, "rb") as f:
            data = f.read()
        self.writestr(arcname or filename, data, compress_type, compresslevel)

    def close(self):
        if self.fp is None:
            return
        if self.mode in "wax":
            start = self.fp.tell()
            for z in self.filelist:
                name = z.filename.encode("utf-8")
                d, t = z._dostime()
                self.fp.write(struct.pack("<4sBBBBHHHHIIIHHHHHII", b"PK\x01\x02", z.create_version, z.create_system, z.extract_version, z.reserved, z.flag_bits, z.compress_type, t, d, z.CRC, z.compress_size, z.file_size, len(name), len(z.extra), len(z.comment), 0, z.internal_attr, z.external_attr, z.header_offset))
                self.fp.write(name)
                self.fp.write(z.extra)
                self.fp.write(z.comment)
            end = self.fp.tell()
            self.fp.write(struct.pack("<4sHHHHIIH", b"PK\x05\x06", 0, 0, len(self.filelist), len(self.filelist), end - start, start, len(self.comment)))
            self.fp.write(self.comment)
        if not self._filePassed:
            self.fp.close()
        self.fp = None

    def __enter__(self):
        return self

    def __exit__(self, *a):
        self.close()


def is_zipfile(filename):
    try:
        ZipFile(filename).close()
        return True
    except Exception:
        return False
"##;

const GZIP: &str = r##"
import io, struct, zlib, time

READ, WRITE = 1, 2


class BadGzipFile(OSError):
    pass


def compress(data, compresslevel=9, mtime=None):
    if mtime is None:
        mtime = time.time()
    c = zlib.compressobj(compresslevel, zlib.DEFLATED, -15)
    body = c.compress(data) + c.flush()
    head = b"\x1f\x8b\x08\x00" + struct.pack("<I", int(mtime) & 0xffffffff) + b"\x02\xff"
    return head + body + struct.pack("<II", zlib.crc32(data) & 0xffffffff, len(data) & 0xffffffff)


def decompress(data):
    out = []
    while data:
        if data[:2] != b"\x1f\x8b":
            raise BadGzipFile("Not a gzipped file (%r)" % data[:2])
        flag = bytearray(data[3:4])[0]
        p = 10
        if flag & 4:
            xlen = struct.unpack("<H", data[p:p + 2])[0]
            p += 2 + xlen
        if flag & 8:
            p = data.index(b"\0", p) + 1
        if flag & 16:
            p = data.index(b"\0", p) + 1
        if flag & 2:
            p += 2
        d = zlib.decompressobj(-15)
        out.append(d.decompress(data[p:]))
        rest = d.unused_data
        data = rest[8:]
    return b"".join(out)


class GzipFile(io.BytesIO):
    def __init__(self, filename=None, mode=None, compresslevel=9, fileobj=None, mtime=None):
        mode = mode or getattr(fileobj, "mode", "rb")
        self._fileobj = fileobj
        self._own = False
        if fileobj is None:
            fileobj = io.open(filename, mode if "b" in mode else mode + "b")
            self._own = True
        self.fileobj = fileobj
        self.compresslevel = compresslevel
        self.mtime = mtime
        if mode.startswith("r"):
            self.mode = READ
            io.BytesIO.__init__(self, decompress(fileobj.read()))
        else:
            self.mode = WRITE
            io.BytesIO.__init__(self)

    def close(self):
        if self.fileobj is None:
            return
        if self.mode == WRITE:
            self.fileobj.write(compress(self.getvalue(), self.compresslevel, self.mtime))
        if self._own:
            self.fileobj.close()
        self.fileobj = None


def open(filename, mode="rb", compresslevel=9, encoding=None, errors=None, newline=None):
    f = GzipFile(filename if isinstance(filename, (str, bytes)) else None, mode.replace("t", ""), compresslevel, None if isinstance(filename, (str, bytes)) else filename)
    if "t" in mode:
        return io.TextIOWrapper(f, encoding, errors, newline)
    return f
"##;

const SOCKET: &str = r##"
AF_INET = 2
AF_INET6 = 30
SOCK_STREAM = 1
SOCK_DGRAM = 2
SOL_SOCKET = 0xffff
SO_REUSEADDR = 4
IPPROTO_TCP = 6
_GLOBAL_DEFAULT_TIMEOUT = object()


class error(OSError):
    pass


class herror(error):
    pass


class gaierror(error):
    pass


class timeout(error):
    pass


_default_timeout = None


def getdefaulttimeout():
    return _default_timeout


def setdefaulttimeout(t):
    global _default_timeout
    _default_timeout = t


def gethostname():
    return "localhost"


def _unavailable():
    return error("network access is not available in this runtime")


class socket(object):
    def __init__(self, family=AF_INET, type=SOCK_STREAM, proto=0, fileno=None):
        raise _unavailable()


def create_connection(address, timeout=None, source_address=None):
    raise _unavailable()


def getaddrinfo(*args, **kwargs):
    raise gaierror("network access is not available in this runtime")


def gethostbyname(name):
    raise gaierror("network access is not available in this runtime")
"##;

const SIGNAL: &str = r##"
SIGINT = 2
SIGTERM = 15
SIGHUP = 1
SIGKILL = 9
SIGALRM = 14
SIG_DFL = 0
SIG_IGN = 1
_handlers = {}


def signal(signalnum, handler):
    old = _handlers.get(signalnum, SIG_DFL)
    _handlers[signalnum] = handler
    return old


def getsignal(signalnum):
    return _handlers.get(signalnum, SIG_DFL)


def default_int_handler(*args):
    raise KeyboardInterrupt()
"##;

const HTML: &str = r##"
_escapes = {"&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#x27;"}
_named = {"amp": "&", "lt": "<", "gt": ">", "quot": '"', "apos": "'", "nbsp": "\xa0"}


def escape(s, quote=True):
    s = s.replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;")
    if quote:
        s = s.replace('"', "&quot;").replace("'", "&#x27;")
    return s


def unescape(s):
    if "&" not in s:
        return s
    out = []
    i = 0
    n = len(s)
    while i < n:
        c = s[i]
        if c != "&":
            out.append(c)
            i += 1
            continue
        j = s.find(";", i + 1, i + 34)
        if j < 0:
            out.append(c)
            i += 1
            continue
        ent = s[i + 1:j]
        try:
            if ent[:2] in ("#x", "#X"):
                out.append(chr(int(ent[2:], 16)))
            elif ent[:1] == "#":
                out.append(chr(int(ent[1:])))
            elif ent in _named:
                out.append(_named[ent])
            else:
                out.append(s[i:j + 1])
        except ValueError:
            out.append(s[i:j + 1])
        i = j + 1
    return "".join(out)
"##;

const AST: &str = r##"
import re as _re

_token = _re.compile(r'''\s*(?:(?P<str>[rRbBuU]{0,2}(?:"""(?:\\.|[^\\])*?"""|\'\'\'(?:\\.|[^\\])*?\'\'\'|"(?:\\.|[^"\\\n])*"|'(?:\\.|[^'\\\n])*'))|(?P<num>[0-9][0-9a-fA-FxXoObB_.jJeE+-]*|\.[0-9][0-9_eEjJ+-]*)|(?P<name>[A-Za-z_][A-Za-z_0-9]*)|(?P<op>[-+(){}\[\],:]))''')

_ALLOWED_NAMES = ("True", "False", "None", "set")


def _check(source):
    pos = 0
    prev = None
    n = len(source)
    while pos < n:
        if source[pos:].strip() == "":
            break
        m = _token.match(source, pos)
        if not m or m.end() == pos:
            raise ValueError("malformed node or string: %r" % source[pos:pos + 20])
        kind = m.lastgroup
        text = m.group(kind)
        if kind == "name" and text not in _ALLOWED_NAMES:
            raise ValueError("malformed node or string: %r" % text)
        if kind == "name" and text == "set":
            rest = source[m.end():].lstrip()
            if not rest.startswith("()"):
                raise ValueError("malformed node or string: %r" % text)
        if kind == "op" and text == "(" and prev in ("name", "str", "num", ")"):
            if not (prev == "name" and source[:m.start()].rstrip().endswith("set")):
                raise ValueError("malformed node or string: call")
        prev = ")" if (kind == "op" and text in ")]}") else kind
        pos = m.end()


def literal_eval(node_or_string):
    """Evaluates a Python literal (strings, bytes, numbers, tuples, lists,
    dicts, sets, booleans, None); anything else raises ValueError."""
    if not isinstance(node_or_string, str):
        raise ValueError("literal_eval accepts source strings only in this runtime")
    source = node_or_string.strip()
    _check(source)
    return eval(source, {"__builtins__": {"set": set}}, {})
"##;
