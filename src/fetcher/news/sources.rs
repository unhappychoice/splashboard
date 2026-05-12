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
    Politics,
    Photography,
    Entertainment,
    Music,
    Crypto,
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
            Self::Politics => "politics",
            Self::Photography => "photography",
            Self::Entertainment => "entertainment",
            Self::Music => "music",
            Self::Crypto => "crypto",
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
    // ========== Batch 2 expansion ==========
    // ---------- General (cont.) ----------
    NewsSource {
        name: "news_france24",
        display: "France 24",
        category: NewsCategory::General,
        description: "France 24 English-language international news.",
        feeds: &[NewsFeed {
            key: "main",
            url: "https://www.france24.com/en/rss",
            label: "Latest",
        }],
    },
    NewsSource {
        name: "news_dw",
        display: "Deutsche Welle",
        category: NewsCategory::General,
        description: "Deutsche Welle English — Germany's international broadcaster.",
        feeds: &[NewsFeed {
            key: "main",
            url: "https://rss.dw.com/rdf/rss-en-all",
            label: "All news",
        }],
    },
    NewsSource {
        name: "news_sky_news",
        display: "Sky News",
        category: NewsCategory::General,
        description: "Sky News world headlines.",
        feeds: &[NewsFeed {
            key: "world",
            url: "https://feeds.skynews.com/feeds/rss/world.xml",
            label: "World",
        }],
    },
    NewsSource {
        name: "news_cbs",
        display: "CBS News",
        category: NewsCategory::General,
        description: "CBS News top stories.",
        feeds: &[NewsFeed {
            key: "main",
            url: "https://www.cbsnews.com/latest/rss/main",
            label: "Latest",
        }],
    },
    // ---------- Tech (cont.) ----------
    NewsSource {
        name: "news_the_register",
        display: "The Register",
        category: NewsCategory::Tech,
        description: "The Register — UK-based enterprise IT news with a distinctive voice.",
        feeds: &[NewsFeed {
            key: "main",
            url: "https://www.theregister.com/headlines.atom",
            label: "Headlines",
        }],
    },
    NewsSource {
        name: "news_zdnet",
        display: "ZDNET",
        category: NewsCategory::Tech,
        description: "ZDNET — enterprise technology news, reviews, and analysis.",
        feeds: &[NewsFeed {
            key: "main",
            url: "https://www.zdnet.com/news/rss.xml",
            label: "Latest",
        }],
    },
    NewsSource {
        name: "news_slashdot",
        display: "Slashdot",
        category: NewsCategory::Tech,
        description: "Slashdot — news for nerds, stuff that matters.",
        feeds: &[NewsFeed {
            key: "main",
            url: "http://rss.slashdot.org/Slashdot/slashdotMain",
            label: "Main",
        }],
    },
    NewsSource {
        name: "news_venturebeat",
        display: "VentureBeat",
        category: NewsCategory::Tech,
        description: "VentureBeat — tech industry news with a startup-funding lens.",
        feeds: &[NewsFeed {
            key: "main",
            url: "https://venturebeat.com/feed/",
            label: "Latest",
        }],
    },
    // ---------- Gadget (cont.) ----------
    NewsSource {
        name: "news_techradar",
        display: "TechRadar",
        category: NewsCategory::Gadget,
        description: "TechRadar — consumer technology product news and reviews.",
        feeds: &[NewsFeed {
            key: "main",
            url: "https://www.techradar.com/rss",
            label: "Latest",
        }],
    },
    // ---------- Business (cont.) ----------
    NewsSource {
        name: "news_forbes",
        display: "Forbes",
        category: NewsCategory::Business,
        description: "Forbes Business section — companies, markets, and entrepreneurship.",
        feeds: &[NewsFeed {
            key: "business",
            url: "https://www.forbes.com/business/feed/",
            label: "Business",
        }],
    },
    NewsSource {
        name: "news_economist",
        display: "The Economist",
        category: NewsCategory::Business,
        description: "The Economist Business section.",
        feeds: &[NewsFeed {
            key: "business",
            url: "https://www.economist.com/business/rss.xml",
            label: "Business",
        }],
    },
    NewsSource {
        name: "news_fortune",
        display: "Fortune",
        category: NewsCategory::Business,
        description: "Fortune — corporate strategy, executives, and market trends.",
        feeds: &[NewsFeed {
            key: "main",
            url: "https://fortune.com/feed/",
            label: "Latest",
        }],
    },
    // ---------- Science (cont.) ----------
    NewsSource {
        name: "news_new_scientist",
        display: "New Scientist",
        category: NewsCategory::Science,
        description: "New Scientist — magazine-style coverage of science and technology.",
        feeds: &[NewsFeed {
            key: "main",
            url: "https://www.newscientist.com/feed/home/",
            label: "Home",
        }],
    },
    NewsSource {
        name: "news_sciencedaily",
        display: "ScienceDaily",
        category: NewsCategory::Science,
        description: "ScienceDaily — research news across all scientific fields.",
        feeds: &[NewsFeed {
            key: "main",
            url: "https://www.sciencedaily.com/rss/all.xml",
            label: "All news",
        }],
    },
    NewsSource {
        name: "news_phys_org",
        display: "Phys.org",
        category: NewsCategory::Science,
        description: "Phys.org — physics, materials science, and broader hard-science coverage.",
        feeds: &[NewsFeed {
            key: "main",
            url: "https://phys.org/rss-feed/",
            label: "Latest",
        }],
    },
    // ---------- Security (cont.) ----------
    NewsSource {
        name: "news_the_hacker_news",
        display: "The Hacker News",
        category: NewsCategory::Security,
        description: "The Hacker News — daily cybersecurity news and analysis.",
        feeds: &[NewsFeed {
            key: "main",
            url: "https://feeds.feedburner.com/TheHackersNews",
            label: "Latest",
        }],
    },
    NewsSource {
        name: "news_threatpost",
        display: "Threatpost",
        category: NewsCategory::Security,
        description: "Threatpost — vulnerability and malware reporting.",
        feeds: &[NewsFeed {
            key: "main",
            url: "https://threatpost.com/feed/",
            label: "Latest",
        }],
    },
    NewsSource {
        name: "news_dark_reading",
        display: "Dark Reading",
        category: NewsCategory::Security,
        description: "Dark Reading — enterprise security news and analysis.",
        feeds: &[NewsFeed {
            key: "main",
            url: "https://www.darkreading.com/rss.xml",
            label: "Latest",
        }],
    },
    NewsSource {
        name: "news_graham_cluley",
        display: "Graham Cluley",
        category: NewsCategory::Security,
        description: "Graham Cluley's security blog — practical, accessible coverage of threats and breaches.",
        feeds: &[NewsFeed {
            key: "main",
            url: "https://grahamcluley.com/feed/",
            label: "Latest",
        }],
    },
    // ---------- Linux / FOSS (cont.) ----------
    NewsSource {
        name: "news_its_foss",
        display: "It's FOSS",
        category: NewsCategory::Linux,
        description: "It's FOSS — Linux, open-source applications, and tutorials.",
        feeds: &[NewsFeed {
            key: "main",
            url: "https://itsfoss.com/feed/",
            label: "Latest",
        }],
    },
    NewsSource {
        name: "news_distrowatch",
        display: "DistroWatch",
        category: NewsCategory::Linux,
        description: "DistroWatch — Linux distribution release announcements and database updates.",
        feeds: &[NewsFeed {
            key: "main",
            url: "http://distrowatch.com/news/dwd.xml",
            label: "Weekly",
        }],
    },
    // ---------- Gaming (cont.) ----------
    NewsSource {
        name: "news_kotaku",
        display: "Kotaku",
        category: NewsCategory::Gaming,
        description: "Kotaku — video-game news, criticism, and culture writing.",
        feeds: &[NewsFeed {
            key: "main",
            url: "https://kotaku.com/rss",
            label: "Latest",
        }],
    },
    NewsSource {
        name: "news_ign",
        display: "IGN",
        category: NewsCategory::Gaming,
        description: "IGN — broad video-game and entertainment news with reviews.",
        feeds: &[NewsFeed {
            key: "main",
            url: "https://feeds.ign.com/ign/games-all",
            label: "Games",
        }],
    },
    NewsSource {
        name: "news_pc_gamer",
        display: "PC Gamer",
        category: NewsCategory::Gaming,
        description: "PC Gamer — PC-focused gaming news, hardware reviews, and previews.",
        feeds: &[NewsFeed {
            key: "main",
            url: "https://www.pcgamer.com/rss/",
            label: "Latest",
        }],
    },
    NewsSource {
        name: "news_vg247",
        display: "VG247",
        category: NewsCategory::Gaming,
        description: "VG247 — video-game news, deals, and guides.",
        feeds: &[NewsFeed {
            key: "main",
            url: "https://www.vg247.com/feed/articles",
            label: "Articles",
        }],
    },
    // ---------- AI / ML (cont.) ----------
    NewsSource {
        name: "news_hugging_face_blog",
        display: "Hugging Face Blog",
        category: NewsCategory::Ai,
        description: "Hugging Face — research, tooling, and model-release write-ups from the HF team and community.",
        feeds: &[NewsFeed {
            key: "main",
            url: "https://huggingface.co/blog/feed.xml",
            label: "Latest",
        }],
    },
    NewsSource {
        name: "news_the_gradient",
        display: "The Gradient",
        category: NewsCategory::Ai,
        description: "The Gradient — long-form essays on AI / ML research.",
        feeds: &[NewsFeed {
            key: "main",
            url: "https://thegradient.pub/rss/",
            label: "Latest",
        }],
    },
    NewsSource {
        name: "news_mit_tr_ai",
        display: "MIT Technology Review AI",
        category: NewsCategory::Ai,
        description: "MIT Technology Review — AI coverage subset of the broader publication.",
        feeds: &[NewsFeed {
            key: "main",
            url: "https://www.technologyreview.com/feed/?topic=artificial-intelligence",
            label: "AI topic",
        }],
    },
    // ---------- Hardware (cont.) ----------
    NewsSource {
        name: "news_techpowerup",
        display: "TechPowerUp",
        category: NewsCategory::Hardware,
        description: "TechPowerUp — PC component news, driver releases, and database updates.",
        feeds: &[NewsFeed {
            key: "main",
            url: "https://www.techpowerup.com/rss/news",
            label: "News",
        }],
    },
    NewsSource {
        name: "news_ifixit",
        display: "iFixit Blog",
        category: NewsCategory::Hardware,
        description: "iFixit Blog — repairability news, teardowns, and right-to-repair advocacy.",
        feeds: &[NewsFeed {
            key: "main",
            url: "https://www.ifixit.com/blog/feed",
            label: "Latest",
        }],
    },
    // ---------- Web / design (cont.) ----------
    NewsSource {
        name: "news_a_list_apart",
        display: "A List Apart",
        category: NewsCategory::Web,
        description: "A List Apart — long-running web standards and design writing.",
        feeds: &[NewsFeed {
            key: "main",
            url: "https://alistapart.com/main/feed",
            label: "Latest",
        }],
    },
    NewsSource {
        name: "news_ux_collective",
        display: "UX Collective",
        category: NewsCategory::Web,
        description: "UX Collective — Medium publication on UX design, research, and practice.",
        feeds: &[NewsFeed {
            key: "main",
            url: "https://uxdesign.cc/feed",
            label: "Latest",
        }],
    },
    // ---------- Apple (cont.) ----------
    NewsSource {
        name: "news_daring_fireball",
        display: "Daring Fireball",
        category: NewsCategory::Apple,
        description: "Daring Fireball — John Gruber's long-running blog on Apple, design, and software.",
        feeds: &[NewsFeed {
            key: "main",
            url: "https://daringfireball.net/feeds/main",
            label: "Main",
        }],
    },
    NewsSource {
        name: "news_appleinsider",
        display: "AppleInsider",
        category: NewsCategory::Apple,
        description: "AppleInsider — Apple ecosystem news, rumors, and reviews.",
        feeds: &[NewsFeed {
            key: "main",
            url: "https://appleinsider.com/rss/news/",
            label: "News",
        }],
    },
    // ---------- Android (cont.) ----------
    NewsSource {
        name: "news_android_authority",
        display: "Android Authority",
        category: NewsCategory::Android,
        description: "Android Authority — Android phones, reviews, and Google ecosystem coverage.",
        feeds: &[NewsFeed {
            key: "main",
            url: "https://www.androidauthority.com/feed/",
            label: "Latest",
        }],
    },
    NewsSource {
        name: "news_xda",
        display: "XDA Developers",
        category: NewsCategory::Android,
        description: "XDA Developers — Android customization, ROMs, and developer-focused phone news.",
        feeds: &[NewsFeed {
            key: "main",
            url: "https://www.xda-developers.com/feed/",
            label: "Latest",
        }],
    },
    // ---------- Space (cont.) ----------
    NewsSource {
        name: "news_nasa_news",
        display: "NASA News",
        category: NewsCategory::Space,
        description: "NASA breaking-news feed — mission updates, science releases, agency announcements.",
        feeds: &[NewsFeed {
            key: "main",
            url: "https://www.nasa.gov/rss/dyn/breaking_news.rss",
            label: "Breaking news",
        }],
    },
    NewsSource {
        name: "news_space_com",
        display: "Space.com",
        category: NewsCategory::Space,
        description: "Space.com — space news, astronomy, and skywatching guides.",
        feeds: &[NewsFeed {
            key: "main",
            url: "https://www.space.com/feeds/all",
            label: "All",
        }],
    },
    // ---------- Climate (cont.) ----------
    NewsSource {
        name: "news_carbon_brief",
        display: "Carbon Brief",
        category: NewsCategory::Climate,
        description: "Carbon Brief — climate science, policy, and energy reporting.",
        feeds: &[NewsFeed {
            key: "main",
            url: "https://www.carbonbrief.org/feed/",
            label: "Latest",
        }],
    },
    // ---------- Politics ----------
    NewsSource {
        name: "news_politico",
        display: "Politico",
        category: NewsCategory::Politics,
        description: "Politico — US politics and policy reporting.",
        feeds: &[NewsFeed {
            key: "main",
            url: "https://rss.politico.com/politics-news.xml",
            label: "Politics",
        }],
    },
    NewsSource {
        name: "news_propublica",
        display: "ProPublica",
        category: NewsCategory::Politics,
        description: "ProPublica — investigative journalism on US institutions and policy.",
        feeds: &[NewsFeed {
            key: "main",
            url: "https://www.propublica.org/feeds/propublica/main",
            label: "Main",
        }],
    },
    NewsSource {
        name: "news_vox",
        display: "Vox",
        category: NewsCategory::Politics,
        description: "Vox — explanatory journalism on politics, policy, and culture.",
        feeds: &[NewsFeed {
            key: "main",
            url: "https://www.vox.com/rss/index.xml",
            label: "Latest",
        }],
    },
    // ---------- Photography ----------
    NewsSource {
        name: "news_dpreview",
        display: "DPReview",
        category: NewsCategory::Photography,
        description: "DPReview — digital camera reviews, gear announcements, and industry news.",
        feeds: &[NewsFeed {
            key: "main",
            url: "https://www.dpreview.com/feeds/news.xml",
            label: "News",
        }],
    },
    NewsSource {
        name: "news_petapixel",
        display: "PetaPixel",
        category: NewsCategory::Photography,
        description: "PetaPixel — photography news, tutorials, and gear coverage.",
        feeds: &[NewsFeed {
            key: "main",
            url: "https://petapixel.com/feed/",
            label: "Latest",
        }],
    },
    NewsSource {
        name: "news_fstoppers",
        display: "Fstoppers",
        category: NewsCategory::Photography,
        description: "Fstoppers — professional photography and videography community.",
        feeds: &[NewsFeed {
            key: "main",
            url: "https://fstoppers.com/rss.xml",
            label: "Latest",
        }],
    },
    // ---------- Entertainment ----------
    NewsSource {
        name: "news_variety",
        display: "Variety",
        category: NewsCategory::Entertainment,
        description: "Variety — entertainment industry news (film, TV, music, theater).",
        feeds: &[NewsFeed {
            key: "main",
            url: "https://variety.com/feed/",
            label: "Latest",
        }],
    },
    NewsSource {
        name: "news_deadline",
        display: "Deadline",
        category: NewsCategory::Entertainment,
        description: "Deadline — Hollywood breaking news and box office reporting.",
        feeds: &[NewsFeed {
            key: "main",
            url: "https://deadline.com/feed/",
            label: "Latest",
        }],
    },
    NewsSource {
        name: "news_indiewire",
        display: "IndieWire",
        category: NewsCategory::Entertainment,
        description: "IndieWire — independent film, festival, and award-season coverage.",
        feeds: &[NewsFeed {
            key: "main",
            url: "https://www.indiewire.com/feed/",
            label: "Latest",
        }],
    },
    // ---------- Music ----------
    NewsSource {
        name: "news_pitchfork",
        display: "Pitchfork",
        category: NewsCategory::Music,
        description: "Pitchfork — music criticism, news, and reviews.",
        feeds: &[NewsFeed {
            key: "news",
            url: "https://pitchfork.com/rss/news/",
            label: "News",
        }],
    },
    NewsSource {
        name: "news_stereogum",
        display: "Stereogum",
        category: NewsCategory::Music,
        description: "Stereogum — indie / alternative music news and commentary.",
        feeds: &[NewsFeed {
            key: "main",
            url: "https://www.stereogum.com/feed/",
            label: "Latest",
        }],
    },
    // ---------- Crypto ----------
    NewsSource {
        name: "news_coindesk",
        display: "CoinDesk",
        category: NewsCategory::Crypto,
        description: "CoinDesk — cryptocurrency and blockchain industry news.",
        feeds: &[NewsFeed {
            key: "main",
            url: "https://www.coindesk.com/arc/outboundfeeds/rss/",
            label: "Latest",
        }],
    },
    NewsSource {
        name: "news_the_block",
        display: "The Block",
        category: NewsCategory::Crypto,
        description: "The Block — investigative crypto industry journalism.",
        feeds: &[NewsFeed {
            key: "main",
            url: "https://www.theblock.co/rss.xml",
            label: "Latest",
        }],
    },
    NewsSource {
        name: "news_decrypt",
        display: "Decrypt",
        category: NewsCategory::Crypto,
        description: "Decrypt — crypto, Web3, and AI news with a consumer lens.",
        feeds: &[NewsFeed {
            key: "main",
            url: "https://decrypt.co/feed",
            label: "Latest",
        }],
    },
];
