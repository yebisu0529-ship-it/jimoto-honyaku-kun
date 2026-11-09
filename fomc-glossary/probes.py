"""FOMC-statement-style probe sentences for terms that the recorded video lacks.
They are written in the style of FOMC statements and press conferences; they are
not captions. Each lists the terms it checks."""
import json, os
PROBES = [
 ("In support of its goals, the Committee decided to lower the target range for the federal funds rate by 1/4 percentage point to 3-3/4 to 4 percent.", ["the Committee", "target range", "federal funds rate", "percentage point"]),
 ("The Committee seeks to achieve maximum employment and inflation at the rate of 2 percent over the longer run.", ["maximum employment", "longer run"]),
 ("Longer-term inflation expectations appear to remain well anchored.", ["inflation expectations", "anchored"]),
 ("Our policy stance is now modestly restrictive, and we are well positioned to respond to developments.", ["restrictive", "well positioned"]),
 ("We cut rates by 25 basis points today.", ["basis points", "cut"]),
 ("In the Summary of Economic Projections, the median participant projects two more rate cuts this year.", ["Summary of Economic Projections", "median participant", "rate cuts"]),
 ("The dot plot is not a plan; it is a collection of individual forecasts.", ["dot plot"]),
 ("We decided to slow the pace of balance sheet runoff.", ["balance sheet runoff"]),
 ("Quantitative tightening has gone smoothly, and reserves remain ample.", ["quantitative tightening", "ample reserves"]),
 ("Total PCE prices rose 2.8 percent over the 12 months ending in September; excluding the volatile food and energy categories, core PCE prices rose 2.9 percent.", ["PCE", "core PCE"]),
 ("We think the neutral rate is probably higher now than it was before the pandemic.", ["neutral rate"]),
 ("A soft landing is still possible, but it is not assured.", ["soft landing"]),
 ("We will remain data dependent and make our decisions meeting by meeting.", ["data dependent", "meeting by meeting"]),
 ("We are not on a preset course.", ["preset course"]),
 ("Downside risks to employment have risen, while upside risks to inflation remain.", ["downside risks", "upside risks"]),
 ("Higher tariffs are pushing up prices in some categories of goods.", ["tariffs"]),
 ("Initial jobless claims remain low, and payroll gains have slowed.", ["jobless claims", "payroll"]),
 ("The labor market has cooled but remains solid.", ["labor market"]),
 ("Governor Waller dissented in favor of a larger cut.", ["Governor", "dissent"]),
 ("The Board of Governors also approved a cut in the discount rate.", ["Board of Governors", "discount rate"]),
 ("Vice Chair Jefferson spoke about the outlook last week.", ["Vice Chair"]),
 ("The Fed is not behind the curve.", ["the Fed", "behind the curve"]),
 ("Headline inflation has come down, but services inflation is still elevated.", ["headline inflation"]),
 ("The Committee will continue to purchase Treasury bills to maintain an ample supply of reserves.", ["Treasury bills", "reserves"]),
 ("The effects of tariffs on inflation could be transitory.", ["transitory"]),
 ("Thank you, Mr. Chairman. Will the Fed hike rates again this year?", ["Mr. Chairman", "hike"]),
 ("Monetary policy works with long and variable lags.", ["monetary policy", "lags"]),
 ("Financial conditions have eased since the last FOMC meeting.", ["financial conditions", "FOMC meeting"]),
 ("The federal funds rate is now in a range of 3.75 to 4 percent.", ["federal funds rate"]),
 ("We lowered the policy rate by a quarter point.", ["policy rate", "quarter point"]),
 ("Our dual mandate is maximum employment and stable prices.", ["dual mandate", "maximum employment"]),
 ("The mandate was given to us by Congress.", ["mandate"]),
 ("The Federal Reserve remains committed to its price stability goal.", ["Federal Reserve", "price stability"]),
 ("Fed funds futures are pricing in another cut in December.", ["Fed funds futures"]),
 ("Chair Powell said the Committee is not in a hurry.", ["Chair"]),
]
here = os.path.dirname(os.path.abspath(__file__))
json.dump([{"set": "probe", "start_ms": i * 100000, "end_ms": i * 100000 + 1, "source": s, "terms": t}
           for i, (s, t) in enumerate(PROBES)], open(os.path.join(here, "probes.json"), "w", encoding="utf-8"),
          ensure_ascii=False, indent=1)
print(len(PROBES), "probes")
