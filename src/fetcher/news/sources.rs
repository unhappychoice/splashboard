//! Static table of curated news sources for the `news_*` family. Adding a source = appending
//! one `NewsSource` row; registration in `mod.rs` is automatic.
//!
//! Each entry hardcodes its feed URL set. Config picks among `feeds` via the `feed` option key,
//! but cannot inject a new URL — that's what keeps the family Safety::Safe.
//!
//! `feeds[0]` is the default rendered when no `feed` option is set. Sub-feeds are only listed
//! for sources whose sections we've confirmed expose stable RSS endpoints; mid-tier sources
//! typically just have one entry. New sub-feeds slot in via single-row catalog adds.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NewsCategory {
    General,
    Tech,
    Gadget,
    Business,
    Science,
    Security,
    Linux,
    Gaming,
    Ai,
    Hardware,
    Web,
    Apple,
    Android,
    Space,
    Climate,
}

impl NewsCategory {
    pub fn label(self) -> &'static str {
        match self {
            Self::General => "general",
            Self::Tech => "tech",
            Self::Gadget => "gadget",
            Self::Business => "business",
            Self::Science => "science",
            Self::Security => "security",
            Self::Linux => "linux",
            Self::Gaming => "gaming",
            Self::Ai => "ai",
            Self::Hardware => "hardware",
            Self::Web => "web",
            Self::Apple => "apple",
            Self::Android => "android",
            Self::Space => "space",
            Self::Climate => "climate",
        }
    }
}

#[derive(Debug)]
pub struct NewsFeed {
    /// Config key (`feed = "world"`). Must be unique within the source.
    pub key: &'static str,
    pub url: &'static str,
    /// Short human-readable label, used in the description blurb.
    pub label: &'static str,
}

#[derive(Debug)]
pub struct NewsSource {
    /// Full fetcher name, registered as-is (`"news_bbc"`).
    pub name: &'static str,
    pub display: &'static str,
    pub category: NewsCategory,
    /// One-sentence blurb shown in the generated catalog. Mention the source positioning so
    /// config authors can decide between siblings.
    pub description: &'static str,
    /// Hardcoded feed URL set. `[0]` is the default.
    pub feeds: &'static [NewsFeed],
}

impl NewsSource {
    pub fn default_feed(&self) -> &'static NewsFeed {
        &self.feeds[0]
    }

    pub fn find_feed(&self, key: &str) -> Option<&'static NewsFeed> {
        self.feeds.iter().find(|f| f.key == key)
    }
}

pub const SOURCES: &[NewsSource] = &[
    // ---------- General ----------
    NewsSource {
        name: "news_bbc",
        display: "BBC",
        category: NewsCategory::General,
        description: "BBC News headline feed. Sub-feeds cover world, top stories, business, technology, science, and politics.",
        feeds: &[
            NewsFeed {
                key: "world",
                url: "http://feeds.bbci.co.uk/news/world/rss.xml",
                label: "World",
            },
            NewsFeed {
                key: "top",
                url: "http://feeds.bbci.co.uk/news/rss.xml",
                label: "Top stories",
            },
            NewsFeed {
                key: "business",
                url: "http://feeds.bbci.co.uk/news/business/rss.xml",
                label: "Business",
            },
            NewsFeed {
                key: "tech",
                url: "http://feeds.bbci.co.uk/news/technology/rss.xml",
                label: "Technology",
            },
            NewsFeed {
                key: "science",
                url: "http://feeds.bbci.co.uk/news/science_and_environment/rss.xml",
                label: "Science & environment",
            },
            NewsFeed {
                key: "politics",
                url: "http://feeds.bbci.co.uk/news/politics/rss.xml",
                label: "Politics",
            },
        ],
    },
    NewsSource {
        name: "news_guardian",
        display: "The Guardian",
        category: NewsCategory::General,
        description: "The Guardian section feeds. Sub-feeds cover world, UK, US, business, technology, and science.",
        feeds: &[
            NewsFeed {
                key: "world",
                url: "https://www.theguardian.com/world/rss",
                label: "World",
            },
            NewsFeed {
                key: "uk",
                url: "https://www.theguardian.com/uk-news/rss",
                label: "UK news",
            },
            NewsFeed {
                key: "us",
                url: "https://www.theguardian.com/us-news/rss",
                label: "US news",
            },
            NewsFeed {
                key: "business",
                url: "https://www.theguardian.com/uk/business/rss",
                label: "Business",
            },
            NewsFeed {
                key: "tech",
                url: "https://www.theguardian.com/uk/technology/rss",
                label: "Technology",
            },
            NewsFeed {
                key: "science",
                url: "https://www.theguardian.com/science/rss",
                label: "Science",
            },
        ],
    },
    NewsSource {
        name: "news_aljazeera",
        display: "Al Jazeera",
        category: NewsCategory::General,
        description: "Al Jazeera English aggregate news feed.",
        feeds: &[NewsFeed {
            key: "all",
            url: "https://www.aljazeera.com/xml/rss/all.xml",
            label: "All news",
        }],
    },
    NewsSource {
        name: "news_npr",
        display: "NPR",
        category: NewsCategory::General,
        description: "NPR top stories headline feed.",
        feeds: &[NewsFeed {
            key: "top",
            url: "https://feeds.npr.org/1001/rss.xml",
            label: "Top stories",
        }],
    },
    // ---------- Tech ----------
    NewsSource {
        name: "news_arstechnica",
        display: "Ars Technica",
        category: NewsCategory::Tech,
        description: "Ars Technica section feeds. Sub-feeds cover the front page, features, tech policy, science, and gaming culture.",
        feeds: &[
            NewsFeed {
                key: "main",
                url: "https://feeds.arstechnica.com/arstechnica/index",
                label: "Front page",
            },
            NewsFeed {
                key: "features",
                url: "https://feeds.arstechnica.com/arstechnica/features",
                label: "Features",
            },
            NewsFeed {
                key: "tech_policy",
                url: "https://feeds.arstechnica.com/arstechnica/tech-policy",
                label: "Tech policy",
            },
            NewsFeed {
                key: "science",
                url: "https://feeds.arstechnica.com/arstechnica/science",
                label: "Science",
            },
            NewsFeed {
                key: "gaming",
                url: "https://feeds.arstechnica.com/arstechnica/gaming",
                label: "Gaming & culture",
            },
        ],
    },
    NewsSource {
        name: "news_techcrunch",
        display: "TechCrunch",
        category: NewsCategory::Tech,
        description: "TechCrunch startups + venture news. Default is the all-stories feed.",
        feeds: &[
            NewsFeed {
                key: "main",
                url: "https://techcrunch.com/feed/",
                label: "All stories",
            },
            NewsFeed {
                key: "startups",
                url: "https://techcrunch.com/category/startups/feed/",
                label: "Startups",
            },
            NewsFeed {
                key: "venture",
                url: "https://techcrunch.com/category/venture/feed/",
                label: "Venture",
            },
        ],
    },
    NewsSource {
        name: "news_theverge",
        display: "The Verge",
        category: NewsCategory::Tech,
        description: "The Verge section feeds. Sub-feeds cover the front page, gaming, tech, entertainment, and science.",
        feeds: &[
            NewsFeed {
                key: "main",
                url: "https://www.theverge.com/rss/index.xml",
                label: "Front page",
            },
            NewsFeed {
                key: "tech",
                url: "https://www.theverge.com/rss/tech/index.xml",
                label: "Tech",
            },
            NewsFeed {
                key: "gaming",
                url: "https://www.theverge.com/rss/games/index.xml",
                label: "Gaming",
            },
            NewsFeed {
                key: "entertainment",
                url: "https://www.theverge.com/rss/entertainment/index.xml",
                label: "Entertainment",
            },
            NewsFeed {
                key: "science",
                url: "https://www.theverge.com/rss/science/index.xml",
                label: "Science",
            },
        ],
    },
    NewsSource {
        name: "news_wired",
        display: "WIRED",
        category: NewsCategory::Tech,
        description: "WIRED section feeds. Sub-feeds cover the front page, business, culture, gear, science, and security.",
        feeds: &[
            NewsFeed {
                key: "main",
                url: "https://www.wired.com/feed/rss",
                label: "Front page",
            },
            NewsFeed {
                key: "business",
                url: "https://www.wired.com/feed/category/business/latest/rss",
                label: "Business",
            },
            NewsFeed {
                key: "culture",
                url: "https://www.wired.com/feed/category/culture/latest/rss",
                label: "Culture",
            },
            NewsFeed {
                key: "gear",
                url: "https://www.wired.com/feed/category/gear/latest/rss",
                label: "Gear",
            },
            NewsFeed {
                key: "science",
                url: "https://www.wired.com/feed/category/science/latest/rss",
                label: "Science",
            },
            NewsFeed {
                key: "security",
                url: "https://www.wired.com/feed/category/security/latest/rss",
                label: "Security",
            },
        ],
    },
    NewsSource {
        name: "news_hackernoon",
        display: "HackerNoon",
        category: NewsCategory::Tech,
        description: "HackerNoon's site-wide latest stories.",
        feeds: &[NewsFeed {
            key: "main",
            url: "https://hackernoon.com/feed",
            label: "Latest",
        }],
    },
    // ---------- Gadget ----------
    NewsSource {
        name: "news_engadget",
        display: "Engadget",
        category: NewsCategory::Gadget,
        description: "Engadget consumer-electronics headlines.",
        feeds: &[NewsFeed {
            key: "main",
            url: "https://www.engadget.com/rss.xml",
            label: "All stories",
        }],
    },
    NewsSource {
        name: "news_gizmodo",
        display: "Gizmodo",
        category: NewsCategory::Gadget,
        description: "Gizmodo consumer-electronics + nerd-culture headlines.",
        feeds: &[NewsFeed {
            key: "main",
            url: "https://gizmodo.com/rss",
            label: "All stories",
        }],
    },
    // ---------- Business ----------
    NewsSource {
        name: "news_ft",
        display: "Financial Times",
        category: NewsCategory::Business,
        description: "Financial Times technology section, the FT subset that doesn't require a subscription to read in-app.",
        feeds: &[NewsFeed {
            key: "tech",
            url: "https://www.ft.com/technology?format=rss",
            label: "Technology",
        }],
    },
    NewsSource {
        name: "news_marketwatch",
        display: "MarketWatch",
        category: NewsCategory::Business,
        description: "MarketWatch top stories — US-leaning market news.",
        feeds: &[NewsFeed {
            key: "top",
            url: "https://feeds.content.dowjones.io/public/rss/mw_topstories",
            label: "Top stories",
        }],
    },
    NewsSource {
        name: "news_yahoo_finance",
        display: "Yahoo Finance",
        category: NewsCategory::Business,
        description: "Yahoo Finance — US-leaning aggregated market and business headlines.",
        feeds: &[NewsFeed {
            key: "main",
            url: "https://finance.yahoo.com/news/rssindex",
            label: "Latest",
        }],
    },
    // ---------- Science ----------
    NewsSource {
        name: "news_nature",
        display: "Nature",
        category: NewsCategory::Science,
        description: "Nature, the weekly international science journal.",
        feeds: &[NewsFeed {
            key: "main",
            url: "https://www.nature.com/nature.rss",
            label: "Latest",
        }],
    },
    NewsSource {
        name: "news_scientific_american",
        display: "Scientific American",
        category: NewsCategory::Science,
        description: "Scientific American global English-language feed.",
        feeds: &[NewsFeed {
            key: "main",
            url: "https://www.scientificamerican.com/platform/syndication/rss/",
            label: "Latest",
        }],
    },
    NewsSource {
        name: "news_quanta_magazine",
        display: "Quanta Magazine",
        category: NewsCategory::Science,
        description: "Quanta Magazine, long-form math / physics / biology / computer science journalism.",
        feeds: &[NewsFeed {
            key: "main",
            url: "https://api.quantamagazine.org/feed/",
            label: "Latest",
        }],
    },
    // ---------- Security ----------
    NewsSource {
        name: "news_krebs",
        display: "Krebs on Security",
        category: NewsCategory::Security,
        description: "Brian Krebs's investigative reporting on cybercrime.",
        feeds: &[NewsFeed {
            key: "main",
            url: "https://krebsonsecurity.com/feed/",
            label: "Latest",
        }],
    },
    NewsSource {
        name: "news_bleeping_computer",
        display: "Bleeping Computer",
        category: NewsCategory::Security,
        description: "Bleeping Computer's news desk, focused on malware, ransomware, and patch reporting.",
        feeds: &[NewsFeed {
            key: "main",
            url: "https://www.bleepingcomputer.com/feed/",
            label: "Latest",
        }],
    },
    NewsSource {
        name: "news_schneier",
        display: "Schneier on Security",
        category: NewsCategory::Security,
        description: "Bruce Schneier's blog — long-form security commentary + weekly squid post.",
        feeds: &[NewsFeed {
            key: "main",
            url: "https://www.schneier.com/feed/atom/",
            label: "Latest",
        }],
    },
    // ---------- Linux / FOSS ----------
    NewsSource {
        name: "news_lwn",
        display: "LWN",
        category: NewsCategory::Linux,
        description: "LWN.net headlines — kernel + Linux ecosystem long-form.",
        feeds: &[NewsFeed {
            key: "main",
            url: "https://lwn.net/headlines/rss",
            label: "Headlines",
        }],
    },
    NewsSource {
        name: "news_phoronix",
        display: "Phoronix",
        category: NewsCategory::Linux,
        description: "Phoronix Linux hardware, kernel, and benchmarking news.",
        feeds: &[NewsFeed {
            key: "main",
            url: "https://www.phoronix.com/rss.php",
            label: "Latest",
        }],
    },
    NewsSource {
        name: "news_omg_ubuntu",
        display: "OMG! Ubuntu",
        category: NewsCategory::Linux,
        description: "OMG! Ubuntu — Ubuntu / GNOME desktop app + distro news.",
        feeds: &[NewsFeed {
            key: "main",
            url: "https://www.omgubuntu.co.uk/feed",
            label: "Latest",
        }],
    },
    // ---------- Gaming ----------
    NewsSource {
        name: "news_polygon",
        display: "Polygon",
        category: NewsCategory::Gaming,
        description: "Polygon video-game news + culture coverage.",
        feeds: &[NewsFeed {
            key: "main",
            url: "https://www.polygon.com/rss/index.xml",
            label: "Latest",
        }],
    },
    NewsSource {
        name: "news_eurogamer",
        display: "Eurogamer",
        category: NewsCategory::Gaming,
        description: "Eurogamer video-game news from the UK / EU desk.",
        feeds: &[NewsFeed {
            key: "main",
            url: "https://www.eurogamer.net/feed",
            label: "Latest",
        }],
    },
    NewsSource {
        name: "news_rockpapershotgun",
        display: "Rock Paper Shotgun",
        category: NewsCategory::Gaming,
        description: "Rock Paper Shotgun, PC-game-centric news and features.",
        feeds: &[NewsFeed {
            key: "main",
            url: "https://www.rockpapershotgun.com/feed",
            label: "Latest",
        }],
    },
    // ---------- AI / ML ----------
    NewsSource {
        name: "news_import_ai",
        display: "Import AI",
        category: NewsCategory::Ai,
        description: "Jack Clark's weekly Import AI newsletter, covering policy + research developments.",
        feeds: &[NewsFeed {
            key: "main",
            url: "https://importai.substack.com/feed",
            label: "Issues",
        }],
    },
    NewsSource {
        name: "news_ai_news",
        display: "AI News",
        category: NewsCategory::Ai,
        description: "artificialintelligence-news.com — industry AI / ML news desk, complement to long-form newsletters like Import AI.",
        feeds: &[NewsFeed {
            key: "main",
            url: "https://www.artificialintelligence-news.com/feed/",
            label: "Latest",
        }],
    },
    // ---------- Hardware / makers ----------
    NewsSource {
        name: "news_hackaday",
        display: "Hackaday",
        category: NewsCategory::Hardware,
        description: "Hackaday — DIY electronics + maker projects daily.",
        feeds: &[NewsFeed {
            key: "main",
            url: "https://hackaday.com/feed/",
            label: "Latest",
        }],
    },
    NewsSource {
        name: "news_tomshardware",
        display: "Tom's Hardware",
        category: NewsCategory::Hardware,
        description: "Tom's Hardware — PC component + benchmark news.",
        feeds: &[NewsFeed {
            key: "main",
            url: "https://www.tomshardware.com/feeds/all",
            label: "Latest",
        }],
    },
    // ---------- Web / design ----------
    NewsSource {
        name: "news_smashing_magazine",
        display: "Smashing Magazine",
        category: NewsCategory::Web,
        description: "Smashing Magazine — front-end + UX + accessibility articles.",
        feeds: &[NewsFeed {
            key: "main",
            url: "https://www.smashingmagazine.com/feed/",
            label: "Latest",
        }],
    },
    NewsSource {
        name: "news_css_tricks",
        display: "CSS-Tricks",
        category: NewsCategory::Web,
        description: "CSS-Tricks — CSS, HTML, and JavaScript tips and articles.",
        feeds: &[NewsFeed {
            key: "main",
            url: "https://css-tricks.com/feed/",
            label: "Latest",
        }],
    },
    // ---------- Apple ecosystem ----------
    NewsSource {
        name: "news_9to5mac",
        display: "9to5Mac",
        category: NewsCategory::Apple,
        description: "9to5Mac — Apple ecosystem news (Mac, iPhone, iPad, services).",
        feeds: &[NewsFeed {
            key: "main",
            url: "https://9to5mac.com/feed/",
            label: "Latest",
        }],
    },
    NewsSource {
        name: "news_macrumors",
        display: "MacRumors",
        category: NewsCategory::Apple,
        description: "MacRumors — Apple rumors, news, and product reviews.",
        feeds: &[NewsFeed {
            key: "main",
            url: "https://www.macrumors.com/macrumors.xml",
            label: "Latest",
        }],
    },
    // ---------- Android ----------
    NewsSource {
        name: "news_android_police",
        display: "Android Police",
        category: NewsCategory::Android,
        description: "Android Police — Android phones, apps, and Google product news.",
        feeds: &[NewsFeed {
            key: "main",
            url: "https://www.androidpolice.com/feed/",
            label: "Latest",
        }],
    },
    NewsSource {
        name: "news_9to5google",
        display: "9to5Google",
        category: NewsCategory::Android,
        description: "9to5Google — Android, Pixel, and broader Google service news.",
        feeds: &[NewsFeed {
            key: "main",
            url: "https://9to5google.com/feed/",
            label: "Latest",
        }],
    },
    // ---------- Space / astronomy ----------
    NewsSource {
        name: "news_spacenews",
        display: "SpaceNews",
        category: NewsCategory::Space,
        description: "SpaceNews — commercial + government space industry coverage.",
        feeds: &[NewsFeed {
            key: "main",
            url: "https://spacenews.com/feed/",
            label: "Latest",
        }],
    },
    NewsSource {
        name: "news_universe_today",
        display: "Universe Today",
        category: NewsCategory::Space,
        description: "Universe Today — astronomy + space exploration daily.",
        feeds: &[NewsFeed {
            key: "main",
            url: "https://www.universetoday.com/feed/",
            label: "Latest",
        }],
    },
    // ---------- Climate / environment ----------
    NewsSource {
        name: "news_yale_climate",
        display: "Yale Climate Connections",
        category: NewsCategory::Climate,
        description: "Yale Climate Connections — climate science + policy reporting.",
        feeds: &[NewsFeed {
            key: "main",
            url: "https://yaleclimateconnections.org/feed/",
            label: "Latest",
        }],
    },
    NewsSource {
        name: "news_inside_climate",
        display: "Inside Climate News",
        category: NewsCategory::Climate,
        description: "Inside Climate News — climate investigative journalism.",
        feeds: &[NewsFeed {
            key: "main",
            url: "https://insideclimatenews.org/feed/",
            label: "Latest",
        }],
    },
];
