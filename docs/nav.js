(function () {
  var sidebar = document.getElementById("sidebar");
  if (!sidebar) return;

  var root = sidebar.getAttribute("data-root") || ".";

  var path = window.location.pathname;
  var current = path.split("/").pop() || "index.html";

  var section = "";
  var category = "";
  var parts = path.split("/");
  for (var i = 0; i < parts.length; i++) {
    if (parts[i] === "Developer" && i + 1 < parts.length) {
      section = "developer";
      category = parts[i + 1];
      break;
    }
    if (parts[i] === "Usage") {
      section = "usage";
      break;
    }
  }

  var devSections = [
    { slug: "general", title: "General" },
    { slug: "api", title: "API" },
    { slug: "geom", title: "Geometry" },
    { slug: "tech", title: "Technology" },
    { slug: "verify", title: "Verify" },
    { slug: "solver", title: "Solver" },
    { slug: "constraints", title: "Constraints" },
    { slug: "place", title: "Place" },
    { slug: "route", title: "Route" },
  ];

  var sectionPages = {
    general: [
      { file: "data-oriented-design.html", title: "Data-Oriented Design" },
    ],
    api: [
      { file: "General-API-Design.html", title: "API Design Rules" },
      { file: "Philis-Api-Considerations.html", title: "API Considerations" },
    ],
    constraints: [
      { file: "model.html", title: "Canonical Model & Identity" },
      { file: "technology-coverage.html", title: "PDK Technology & Coverage" },
      { file: "gates-and-quality.html", title: "Gates & Quality" },
      {
        file: "projections-certificate.html",
        title: "Projections & Certificate",
      },
    ],
    place: [
      { file: "Signoff-First-Placement.html", title: "Signoff-First Problem" },
      {
        file: "Placement-Algorithms.html",
        title: "Algorithms & Foundations",
      },
      { file: "Api-Mapping.html", title: "API Mapping & Stubs" },
    ],
    route: [
      { file: "Signoff-First-Routing.html", title: "Signoff-First Problem" },
      { file: "Router-Algorithms.html", title: "Algorithms & Foundations" },
      { file: "Api-Mapping.html", title: "API Mapping & Stubs" },
    ],
  };

  var usagePages = [
    { file: "getting-started.html", title: "Getting Started" },
    { file: "installation.html", title: "Installation" },
    { file: "first-layout.html", title: "Your First Layout" },
  ];

  function li(href, text, active) {
    return (
      "<li><a href=\"" +
      href +
      "\"" +
      (active ? ' class="active"' : "") +
      ">" +
      text +
      "</a></li>"
    );
  }

  var html = "";

  if (section === "developer") {
    html += "<h4>Developer</h4><ul>";
    for (var i = 0; i < devSections.length; i++) {
      var s = devSections[i];
      html += li(
        root + "/Developer/" + s.slug + "/index.html",
        s.title,
        s.slug === category
      );
    }
    html += "</ul>";

    var pages = sectionPages[category];
    if (pages && pages.length > 0) {
      var title = category;
      for (var i = 0; i < devSections.length; i++) {
        if (devSections[i].slug === category) {
          title = devSections[i].title;
          break;
        }
      }
      html += "<h4>" + title + "</h4><ul>";
      html += li(
        root + "/Developer/" + category + "/index.html",
        "Overview",
        current === "index.html"
      );
      for (var j = 0; j < pages.length; j++) {
        html += li(
          root + "/Developer/" + category + "/" + pages[j].file,
          pages[j].title,
          current === pages[j].file
        );
      }
      html += "</ul>";
    }

    html += "<h4>Usage</h4><ul>";
    for (var i = 0; i < usagePages.length; i++) {
      html += li(root + "/Usage/" + usagePages[i].file, usagePages[i].title, false);
    }
    html += "</ul>";
  } else if (section === "usage") {
    html += "<h4>Usage</h4><ul>";
    for (var i = 0; i < usagePages.length; i++) {
      html += li(
        root + "/Usage/" + usagePages[i].file,
        usagePages[i].title,
        current === usagePages[i].file
      );
    }
    html += "</ul>";

    html += "<h4>Developer</h4><ul>";
    for (var i = 0; i < devSections.length; i++) {
      var s = devSections[i];
      html += li(root + "/Developer/" + s.slug + "/index.html", s.title, false);
    }
    html += "</ul>";
  }

  sidebar.innerHTML = html;
})();
