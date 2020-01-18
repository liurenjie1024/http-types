use async_std::io::{self, BufRead, Read};
use async_std::sync;

use std::mem;
use std::pin::Pin;
use std::task::{Context, Poll};

use crate::headers::{
    self, HeaderName, HeaderValue, Headers, Names, ToHeaderValues, Values, CONTENT_TYPE,
};
use crate::mime::Mime;
use crate::Cookie;
use crate::trailers::{Trailers, TrailersSender};
use crate::{Body, Method, Url, Version};

pin_project_lite::pin_project! {
    /// An HTTP request.
    ///
    /// # Examples
    ///
    /// ```
    /// use http_types::{Url, Method, Request};
    ///
    /// let mut req = Request::new(Method::Get, Url::parse("https://example.com").unwrap());
    /// req.set_body("Hello, Nori!");
    /// ```
    #[derive(Debug)]
    pub struct Request {
        method: Method,
        url: Url,
        headers: Headers,
        version: Option<Version>,
        sender: Option<sync::Sender<io::Result<Trailers>>>,
        receiver: sync::Receiver<io::Result<Trailers>>,
        #[pin]
        body: Body,
    }
}

impl Request {
    /// Create a new request.
    pub fn new(method: Method, url: Url) -> Self {
        let (sender, receiver) = sync::channel(1);
        Self {
            method,
            url,
            headers: Headers::new(),
            version: None,
            body: Body::empty(),
            sender: Some(sender),
            receiver,
        }
    }

    /// Get the HTTP method
    pub fn method(&self) -> Method {
        self.method
    }

    /// Set the HTTP method.
    pub fn set_method(&mut self, method: Method) {
        self.method = method;
    }

    /// Set the headers.
    pub fn set_headers<'a, T: IntoIterator<Item = (&'a HeaderName, &'a Vec<HeaderValue>)>>(
        &mut self,
        headers: T,
    ) {
        self.headers = Headers {
            headers: headers
                .into_iter()
                .map(|(name, values)| (name.clone(), values.clone()))
                .collect(),
        };
    }

    /// Get a reference to the url.
    ///
    /// # Examples
    ///
    /// ```
    /// # fn main() -> Result<(), http_types::url::ParseError> {
    /// #
    /// use http_types::{Url, Method, Request, Response, StatusCode};
    /// let mut req = Request::new(Method::Get, Url::parse("https://example.com")?);
    /// assert_eq!(req.url().scheme(), "https");
    /// #
    /// # Ok(()) }
    /// ```
    pub fn url(&self) -> &Url {
        &self.url
    }

    /// Get a mutable reference to the url.
    ///
    /// # Examples
    ///
    /// ```
    /// # fn main() -> Result<(), http_types::url::ParseError> {
    /// #
    /// use http_types::{Url, Method, Request, Response, StatusCode};
    /// let mut req = Request::new(Method::Get, Url::parse("https://example.com")?);
    /// req.url_mut().set_scheme("http");
    /// assert_eq!(req.url().scheme(), "http");
    /// #
    /// # Ok(()) }
    /// ```
    pub fn url_mut(&mut self) -> &mut Url {
        &mut self.url
    }

    /// Set the request body.
    ///
    /// # Examples
    ///
    /// ```
    /// use http_types::{Url, Method, Request};
    ///
    /// let mut req = Request::new(Method::Get, Url::parse("https://example.com").unwrap());
    /// req.set_body("Hello, Nori!");
    /// ```
    pub fn set_body(&mut self, body: impl Into<Body>) {
        self.body = body.into();
        if self.header(&CONTENT_TYPE).is_none() {
            let mime = self.body.take_mime();
            self.set_content_type(mime);
        }
    }

    /// Swaps the value of the body with another body, without deinitializing
    /// either one.
    ///
    /// # Examples
    ///
    /// ```
    /// # use async_std::io::prelude::*;
    /// # fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync + 'static>> {
    /// # async_std::task::block_on(async {
    /// #
    /// use http_types::{Body, Url, Method, Request};
    ///
    /// let mut req = Request::new(Method::Get, Url::parse("https://example.com").unwrap());
    /// req.set_body("Hello, Nori!");
    /// let mut body: Body = req.replace_body("Hello, Chashu!");
    ///
    /// let mut string = String::new();
    /// body.read_to_string(&mut string).await?;
    /// assert_eq!(&string, "Hello, Nori!");
    /// #
    /// # Ok(()) }) }
    /// ```
    pub fn replace_body(&mut self, body: impl Into<Body>) -> Body {
        mem::replace(&mut self.body, body.into())
    }

    /// Replace the request body with a new body, and return the old body.
    ///
    /// # Examples
    ///
    /// ```
    /// # use async_std::io::prelude::*;
    /// # fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync + 'static>> {
    /// # async_std::task::block_on(async {
    /// #
    /// use http_types::{Body, Url, Method, Request};
    ///
    /// let mut req = Request::new(Method::Get, Url::parse("https://example.com").unwrap());
    /// req.set_body("Hello, Nori!");
    /// let mut body = "Hello, Chashu!".into();
    /// req.swap_body(&mut body);
    ///
    /// let mut string = String::new();
    /// body.read_to_string(&mut string).await?;
    /// assert_eq!(&string, "Hello, Nori!");
    /// #
    /// # Ok(()) }) }
    /// ```
    pub fn swap_body(&mut self, body: &mut Body) {
        mem::swap(&mut self.body, body);
    }

    /// Take the request body, replacing it with an empty body.
    ///
    /// # Examples
    ///
    /// ```
    /// # use async_std::io::prelude::*;
    /// # fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync + 'static>> {
    /// # async_std::task::block_on(async {
    /// #
    /// use http_types::{Body, Url, Method, Request};
    ///
    /// let mut req = Request::new(Method::Get, Url::parse("https://example.com").unwrap());
    /// req.set_body("Hello, Nori!");
    /// let mut body: Body = req.take_body();
    ///
    /// let mut string = String::new();
    /// body.read_to_string(&mut string).await?;
    /// assert_eq!(&string, "Hello, Nori!");
    ///
    /// # let mut string = String::new();
    /// # req.read_to_string(&mut string).await?;
    /// # assert_eq!(&string, "");
    /// #
    /// # Ok(()) }) }
    /// ```
    pub fn take_body(&mut self) -> Body {
        self.replace_body(Body::empty())
    }

    /// Get an HTTP header.
    pub fn header(&self, name: &HeaderName) -> Option<&Vec<HeaderValue>> {
        self.headers.get(name)
    }

    /// Get a mutable reference to a header.
    pub fn header_mut(&mut self, name: &HeaderName) -> Option<&mut Vec<HeaderValue>> {
        self.headers.get_mut(name)
    }

    /// Remove a header.
    pub fn remove_header(&mut self, name: &HeaderName) -> Option<Vec<HeaderValue>> {
        self.headers.remove(name)
    }

    /// Set an HTTP header.
    pub fn insert_header(
        &mut self,
        name: HeaderName,
        values: impl ToHeaderValues,
    ) -> io::Result<Option<Vec<HeaderValue>>> {
        self.headers.insert(name, values)
    }

    /// Append a header to the headers.
    ///
    /// Unlike `insert` this function will not override the contents of a header, but insert a
    /// header if there aren't any. Or else append to the existing list of headers.
    pub fn append_header(
        &mut self,
        name: HeaderName,
        values: impl ToHeaderValues,
    ) -> io::Result<()> {
        self.headers.append(name, values)
    }

    /// Set the response MIME.
    // TODO: return a parsed MIME
    pub fn set_content_type(&mut self, mime: Mime) -> Option<Vec<HeaderValue>> {
        let value: HeaderValue = mime.into();

        // A Mime instance is guaranteed to be valid header name.
        self.insert_header(CONTENT_TYPE, value).unwrap()
    }

    /// Get the current content type
    pub fn content_type(&self) -> Option<Mime> {
        self.header(&CONTENT_TYPE)?.last()?.as_str().parse().ok()
    }

    /// Get the length of the body stream, if it has been set.
    ///
    /// This value is set when passing a fixed-size object into as the body. E.g. a string, or a
    /// buffer. Consumers of this API should check this value to decide whether to use `Chunked`
    /// encoding, or set the response length.
    pub fn len(&self) -> Option<usize> {
        self.body.len()
    }

    /// Get the HTTP version, if one has been set.
    ///
    /// # Examples
    ///
    /// ```
    /// use http_types::{Url, Method, Request, Version};
    ///
    /// # fn main() -> Result<(), http_types::url::ParseError> {
    /// #
    /// let mut req = Request::new(Method::Get, Url::parse("https://example.com")?);
    /// assert_eq!(req.version(), None);
    ///
    /// req.set_version(Some(Version::Http2_0));
    /// assert_eq!(req.version(), Some(Version::Http2_0));
    /// #
    /// # Ok(()) }
    /// ```
    pub fn version(&self) -> Option<Version> {
        self.version
    }

    /// Set the HTTP version.
    ///
    /// # Examples
    ///
    /// ```
    /// use http_types::{Url, Method, Request, Version};
    ///
    /// # fn main() -> Result<(), http_types::url::ParseError> {
    /// #
    /// let mut req = Request::new(Method::Get, Url::parse("https://example.com")?);
    /// req.set_version(Some(Version::Http2_0));
    /// #
    /// # Ok(()) }
    /// ```
    pub fn set_version(&mut self, version: Option<Version>) {
        self.version = version;
    }

    /// Get all cookies.
    ///
    /// # Examples
    ///
    /// ```
    /// # fn main() -> Result<(), http_types::url::ParseError> {
    /// #
    /// use http_types::{Cookie, Url, Method, Request, Version};
    ///
    /// let mut req = Request::new(Method::Get, Url::parse("https://example.com")?);
    /// req.set_cookie(Cookie::new("name", "value"));
    /// assert_eq!(req.cookies().unwrap(), vec![Cookie::new("name", "value")]);
    /// #
    /// # Ok(()) }
    /// ```
    pub fn cookies(&self) -> Result<Vec<Cookie<'_>>, cookie::ParseError> {
        match self.header(&headers::COOKIE) {
            None => Ok(vec![]),
            Some(h) => h.iter().try_fold(vec![], |mut acc, h| {
                let cookie = Cookie::parse(h.as_str())?;
                acc.push(cookie);
                Ok(acc)
            }),
        }
    }

    /// Get a cookie by name.
    ///
    /// # Examples
    ///
    /// ```
    /// # fn main() -> Result<(), http_types::url::ParseError> {
    /// #
    /// use http_types::{Cookie, Url, Method, Request, Version};
    ///
    /// let mut req = Request::new(Method::Get, Url::parse("https://example.com")?);
    /// req.set_cookie(Cookie::new("name", "value"));
    /// assert_eq!(req.cookie("name").unwrap(), Some(Cookie::new("name", "value")));
    /// #
    /// # Ok(()) }
    /// ```
    pub fn cookie(&self, name: &str) -> Result<Option<Cookie<'_>>, cookie::ParseError> {
        let cookies = self.cookies()?;
        let cookie = cookies.into_iter().filter(|c| c.name() == name).next();
        Ok(cookie)
    }

    /// Set a cookie.
    ///
    /// This will not override any existing cookies, and uses the `Cookies` header.
    ///
    /// # Examples
    ///
    /// ```
    /// # fn main() -> Result<(), http_types::url::ParseError> {
    /// #
    /// use http_types::{Cookie, Url, Method, Request, Version};
    ///
    /// let mut req = Request::new(Method::Get, Url::parse("https://example.com")?);
    /// req.set_cookie(Cookie::new("name", "value"));
    /// #
    /// # Ok(()) }
    /// ```
    pub fn set_cookie(&mut self, cookie: Cookie<'_>) {
        self.append_header(headers::COOKIE, HeaderValue::from(cookie))
            .unwrap();
    }

    /// Sends trailers to the a receiver.
    pub fn send_trailers(&mut self) -> TrailersSender {
        let sender = self.sender.take().expect("Trailers sender can only be constructed once");
        TrailersSender::new(sender)
    }

    /// Receive trailers from a sender.
    pub async fn recv_trailers(&self) -> Option<io::Result<Trailers>> {
        self.receiver.recv().await
    }

    /// An iterator visiting all header pairs in arbitrary order.
    pub fn iter<'a>(&'a self) -> headers::Iter<'a> {
        self.headers.iter()
    }

    /// An iterator visiting all header pairs in arbitrary order, with mutable references to the
    /// values.
    pub fn iter_mut<'a>(&'a mut self) -> headers::IterMut<'a> {
        self.headers.iter_mut()
    }

    /// An iterator visiting all header names in arbitrary order.
    pub fn header_names<'a>(&'a self) -> Names<'a> {
        self.headers.names()
    }

    /// An iterator visiting all header values in arbitrary order.
    pub fn header_values<'a>(&'a self) -> Values<'a> {
        self.headers.values()
    }
}

impl Read for Request {
    #[allow(missing_doc_code_examples)]
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.body).poll_read(cx, buf)
    }
}

impl BufRead for Request {
    #[allow(missing_doc_code_examples)]
    fn poll_fill_buf(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<&'_ [u8]>> {
        let this = self.project();
        this.body.poll_fill_buf(cx)
    }

    fn consume(mut self: Pin<&mut Self>, amt: usize) {
        Pin::new(&mut self.body).consume(amt)
    }
}

impl AsRef<Headers> for Request {
    fn as_ref(&self) -> &Headers {
        &self.headers
    }
}

impl AsMut<Headers> for Request {
    fn as_mut(&mut self) -> &mut Headers {
        &mut self.headers
    }
}

impl Into<Body> for Request {
    fn into(self) -> Body {
        self.body
    }
}

impl IntoIterator for Request {
    type Item = (HeaderName, Vec<HeaderValue>);
    type IntoIter = headers::IntoIter;

    /// Returns a iterator of references over the remaining items.
    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.headers.into_iter()
    }
}

impl<'a> IntoIterator for &'a Request {
    type Item = (&'a HeaderName, &'a Vec<HeaderValue>);
    type IntoIter = headers::Iter<'a>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.headers.iter()
    }
}

impl<'a> IntoIterator for &'a mut Request {
    type Item = (&'a HeaderName, &'a mut Vec<HeaderValue>);
    type IntoIter = headers::IterMut<'a>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.headers.iter_mut()
    }
}
